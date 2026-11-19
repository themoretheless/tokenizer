use crate::RegionStats;
use crate::bytecode::iterator::{BytecodeIterator, BytecodeSequenceSource};
use crate::bytecode::opcode::Upvalue;
use crate::{RecordMap, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};

/// One tracked mutable cell slot inside a Bytecode VM region frame.
#[derive(Clone, Debug)]
pub(super) enum VmCellSlot {
    /// Local slot still living on the VM stack frame at `frame_depth`.
    Stack { frame_depth: usize, abs_slot: usize },
    /// Local slot that was captured by a closure and converted into a shared upvalue cell.
    Upval(Weak<RefCell<Upvalue>>),
}

/// Active or parked Bytecode VM region (arena) frame.
#[derive(Clone, Debug)]
pub(super) struct VmRegionFrame {
    pub(super) id: u32,
    pub(super) budget: Option<usize>,
    pub(super) live: usize,
    pub(super) stats: usize,
    pub(super) persistent: bool,
    pub(super) strict: bool,
    pub(super) is_arena: bool,
    /// VM `frames.len()` depth when this frame was opened (used to unwind on `?` / errors).
    pub(super) origin_frame_depth: usize,
    /// `Rc::as_ptr` / `Weak::as_ptr` addresses of upvalues blessed by `promote(...)` / `move(...)`.
    pub(super) blessed: HashSet<usize>,
    pub(super) cells: Vec<VmCellSlot>,
}

/// Region/arena ledger owned by a [`super::Vm`].
#[derive(Debug, Default)]
pub(super) struct VmRegions {
    pub(super) active: Vec<VmRegionFrame>,
    pub(super) persistent: Vec<(String, VmRegionFrame)>,
    pub(super) stats: Vec<RegionStats>,
    pub(super) next_id: u32,
    pub(super) reusable_slots: usize,
}

impl VmRegions {
    #[inline]
    pub(super) fn has_active(&self) -> bool {
        !self.active.is_empty()
    }

    #[inline]
    pub(super) fn has_active_or_persistent(&self) -> bool {
        !self.active.is_empty() || !self.persistent.is_empty()
    }

    fn stats_index(&mut self, name: Option<&str>) -> usize {
        match self
            .stats
            .iter()
            .position(|entry| entry.name.as_deref() == name)
        {
            Some(index) => index,
            None => {
                let index = self.stats.len();
                self.stats.push(RegionStats {
                    name: name.map(str::to_owned),
                    ..RegionStats::default()
                });
                index
            }
        }
    }

    fn mint_frame(
        &mut self,
        budget: Option<usize>,
        name: Option<&str>,
        strict: bool,
        is_arena: bool,
        origin_frame_depth: usize,
    ) -> VmRegionFrame {
        self.next_id = self.next_id.checked_add(1).expect("region id overflow");
        let stats = self.stats_index(name);
        self.stats[stats].entries += 1;
        VmRegionFrame {
            id: self.next_id,
            budget,
            live: 0,
            stats,
            persistent: false,
            strict,
            is_arena,
            origin_frame_depth,
            blessed: HashSet::new(),
            cells: Vec::new(),
        }
    }

    pub(super) fn enter_region(
        &mut self,
        budget: Option<usize>,
        name: Option<&str>,
        strict: bool,
        is_arena: bool,
        origin_frame_depth: usize,
    ) -> u32 {
        let frame = self.mint_frame(budget, name, strict, is_arena, origin_frame_depth);
        let id = frame.id;
        self.active.push(frame);
        id
    }

    /// Track a newly declared mutable local slot in the innermost active region.
    pub(super) fn track_local(
        &mut self,
        frame_depth: usize,
        abs_slot: usize,
    ) -> Result<(), &'static str> {
        let Some(frame) = self.active.last_mut() else {
            return Ok(());
        };
        let reused = if self.reusable_slots > 0 {
            self.reusable_slots -= 1;
            1
        } else {
            0
        };
        frame.cells.push(VmCellSlot::Stack {
            frame_depth,
            abs_slot,
        });
        frame.live += 1;
        let stats = &mut self.stats[frame.stats];
        stats.allocated += 1;
        stats.reused_slots += reused;
        stats.peak_live = stats.peak_live.max(frame.live);
        if frame.budget.is_some_and(|budget| frame.live > budget) {
            return Err("Region cell budget exceeded");
        }
        Ok(())
    }

    /// When a stack slot is closed into an `Upvalue`, rewrite its tracked region entry.
    pub(super) fn convert_stack_to_upval(&mut self, abs_slot: usize, upval: &Rc<RefCell<Upvalue>>) {
        for frame in self.active.iter_mut().rev() {
            for entry in frame.cells.iter_mut() {
                if let VmCellSlot::Stack {
                    abs_slot: tracked, ..
                } = entry
                    && *tracked == abs_slot
                {
                    *entry = VmCellSlot::Upval(Rc::downgrade(upval));
                }
            }
        }
    }

    /// Reclaim dead stack slots (in `start..end` of `frame_depth`) and dropped upvalues.
    pub(super) fn sweep_scope(&mut self, frame_depth: usize, start: usize, end: usize) {
        let mut reclaimed = 0usize;
        for frame in self.active.iter_mut() {
            let before = frame.cells.len();
            frame.cells.retain(|entry| match entry {
                VmCellSlot::Stack {
                    frame_depth: fd,
                    abs_slot,
                } => !(*fd == frame_depth && *abs_slot >= start && *abs_slot < end),
                VmCellSlot::Upval(w) => w.strong_count() > 0,
            });
            let freed = before - frame.cells.len();
            frame.live = frame.cells.len();
            reclaimed += freed;
        }
        for (_, frame) in self.persistent.iter_mut() {
            let before = frame.cells.len();
            frame.cells.retain(|entry| match entry {
                VmCellSlot::Stack { .. } => false,
                VmCellSlot::Upval(w) => w.strong_count() > 0,
            });
            let freed = before - frame.cells.len();
            frame.live = frame.cells.len();
            reclaimed += freed;
        }
        self.reusable_slots += reclaimed;
    }

    /// Reclaim any uncaptured stack slots belonging to an exiting call frame.
    pub(super) fn sweep_exiting_frame(&mut self, exiting_depth: usize) {
        let mut reclaimed = 0usize;
        for frame in self.active.iter_mut() {
            let before = frame.cells.len();
            frame.cells.retain(|entry| match entry {
                VmCellSlot::Stack { frame_depth, .. } => *frame_depth != exiting_depth,
                VmCellSlot::Upval(w) => w.strong_count() > 0,
            });
            let freed = before - frame.cells.len();
            frame.live = frame.cells.len();
            reclaimed += freed;
        }
        for (_, frame) in self.persistent.iter_mut() {
            let before = frame.cells.len();
            frame.cells.retain(|entry| match entry {
                VmCellSlot::Stack { .. } => false,
                VmCellSlot::Upval(w) => w.strong_count() > 0,
            });
            let freed = before - frame.cells.len();
            frame.live = frame.cells.len();
            reclaimed += freed;
        }
        self.reusable_slots += reclaimed;
    }

    /// Bless all upvalue cells reachable from `value` so active strict regions allow their escape.
    pub(super) fn bless_vm_value(&mut self, value: &Value<'static>, stack: &[Value<'static>]) {
        if !self.active.iter().any(|frame| frame.strict) {
            return;
        }
        enum Work {
            Val(Value<'static>),
            Upval(Rc<RefCell<Upvalue>>),
        }
        let mut visited: HashSet<(u8, usize)> = HashSet::new();
        let mut work = vec![Work::Val(value.clone())];
        while let Some(item) = work.pop() {
            match item {
                Work::Val(val) => match val {
                    Value::BytecodeFunction(closure) => {
                        if visited.insert((1, Rc::as_ptr(&closure) as usize)) {
                            for upval in &closure.upvalues {
                                work.push(Work::Upval(upval.clone()));
                            }
                        }
                    }
                    Value::BytecodeSequence(seq) => {
                        if visited.insert((2, Rc::as_ptr(&seq) as usize)) {
                            if let BytecodeSequenceSource::List(items) = &seq.source {
                                for item in items {
                                    work.push(Work::Val(item.clone()));
                                }
                            }
                            for stage in &seq.stages {
                                work.push(Work::Val(stage.callback.clone()));
                            }
                        }
                    }
                    Value::BytecodeIterator(cell) => {
                        if visited.insert((4, Rc::as_ptr(&cell) as usize))
                            && let Ok(guard) = cell.try_borrow()
                        {
                            let mut pending_iter = vec![&*guard];
                            while let Some(it) = pending_iter.pop() {
                                match it {
                                    BytecodeIterator::List { items, .. } => {
                                        for item in items {
                                            work.push(Work::Val(item.clone()));
                                        }
                                    }
                                    BytecodeIterator::Range { .. } => {}
                                    BytecodeIterator::Sequence {
                                        source_iter,
                                        stages,
                                        pending,
                                        ..
                                    } => {
                                        pending_iter.push(source_iter);
                                        for stage in stages {
                                            work.push(Work::Val(stage.callback.clone()));
                                        }
                                        for (v, _) in pending {
                                            work.push(Work::Val(v.clone()));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Value::List(items) | Value::Tuple(items) | Value::Variant(_, items) => {
                        for item in items {
                            work.push(Work::Val(item));
                        }
                    }
                    Value::UserData(data) => {
                        for item in data.values {
                            work.push(Work::Val(item));
                        }
                    }
                    Value::Record(fields) => {
                        for item in fields.values_slice() {
                            work.push(Work::Val(item.clone()));
                        }
                    }
                    _ => {}
                },
                Work::Upval(upval) => {
                    let ptr = Rc::as_ptr(&upval) as usize;
                    if !visited.insert((3, ptr)) {
                        continue;
                    }
                    for frame in self.active.iter_mut() {
                        if frame.strict {
                            frame.blessed.insert(ptr);
                        }
                    }
                    let inner = match &*upval.borrow() {
                        Upvalue::Open(slot) => stack.get(*slot).cloned(),
                        Upvalue::Closed(v) => Some(v.clone()),
                    };
                    if let Some(inner) = inner {
                        work.push(Work::Val(inner));
                    }
                }
            }
        }
    }

    pub(super) fn exit_region(&mut self, expected_id: Option<u32>) -> Result<usize, &'static str> {
        let frame = self.active.pop().expect("active VM region frame");
        if let Some(expected) = expected_id {
            debug_assert_eq!(frame.id, expected);
        }
        let mut promoted = 0usize;
        let mut unblessed_promoted = 0usize;
        let mut reclaimed = 0usize;
        let mut survivors = Vec::new();
        for entry in frame.cells {
            match entry {
                VmCellSlot::Stack { .. } => {
                    reclaimed += 1;
                }
                VmCellSlot::Upval(w) => {
                    if w.strong_count() > 0 {
                        let ptr = Weak::as_ptr(&w) as usize;
                        promoted += 1;
                        if !frame.blessed.contains(&ptr) {
                            unblessed_promoted += 1;
                        }
                        survivors.push(VmCellSlot::Upval(w));
                    } else {
                        reclaimed += 1;
                    }
                }
            }
        }
        self.reusable_slots += reclaimed;
        self.stats[frame.stats].promoted += promoted as u64;
        let parent_overflow = if promoted > 0
            && let Some(parent) = self.active.last_mut()
        {
            parent.cells.extend(survivors);
            parent.live += promoted;
            parent.budget.is_some_and(|budget| parent.live > budget)
        } else {
            false
        };
        if frame.strict && unblessed_promoted > 0 {
            return Err(if frame.is_arena {
                "Strict arena cells escaped the run; drop or move() them explicitly"
            } else {
                "Value allocated in a strict region escapes; wrap it in promote(...) to allow the escape"
            });
        }
        if parent_overflow {
            return Err("Region cell budget exceeded");
        }
        Ok(promoted)
    }

    /// Exit any regions opened inside `exiting_depth` (used on `?` early return).
    pub(super) fn exit_regions_to_frame(
        &mut self,
        exiting_depth: usize,
    ) -> Result<(), &'static str> {
        let mut first_err: Option<&'static str> = None;
        while self
            .active
            .last()
            .is_some_and(|frame| frame.origin_frame_depth >= exiting_depth)
        {
            if let Err(msg) = self.exit_region(None)
                && first_err.is_none()
            {
                first_err = Some(msg);
            }
        }
        self.sweep_exiting_frame(exiting_depth);
        if let Some(msg) = first_err {
            Err(msg)
        } else {
            Ok(())
        }
    }

    /// Unwind regions opened at or deeper than `exiting_depth` when an error/frame drops.
    pub(super) fn unwind_to_frame(&mut self, exiting_depth: usize) {
        while self
            .active
            .last()
            .is_some_and(|frame| frame.origin_frame_depth >= exiting_depth)
        {
            let _ = self.exit_region(None);
        }
        self.sweep_exiting_frame(exiting_depth);
    }

    pub(super) fn is_active_persistent(&self, name: &str) -> bool {
        self.active
            .iter()
            .rev()
            .any(|frame| frame.persistent && self.stats[frame.stats].name.as_deref() == Some(name))
    }

    pub(super) fn enter_persistent_arena(
        &mut self,
        name: &str,
        budget: Option<usize>,
        strict: bool,
        origin_frame_depth: usize,
    ) -> (u32, HashSet<usize>) {
        let mut frame = match self.persistent.iter().position(|(n, _)| n == name) {
            Some(index) => {
                let frame = self.persistent.swap_remove(index).1;
                self.stats[frame.stats].entries += 1;
                frame
            }
            None => self.mint_frame(budget, Some(name), strict, true, origin_frame_depth),
        };
        frame.budget = budget;
        frame.persistent = true;
        frame.strict = strict;
        frame.is_arena = true;
        frame.origin_frame_depth = origin_frame_depth;
        frame.blessed.clear();
        frame.cells.retain(|entry| match entry {
            VmCellSlot::Stack { .. } => false,
            VmCellSlot::Upval(w) => w.strong_count() > 0,
        });
        frame.live = frame.cells.len();
        let parked_set: HashSet<usize> = frame
            .cells
            .iter()
            .filter_map(|entry| match entry {
                VmCellSlot::Upval(w) => Some(Weak::as_ptr(w) as usize),
                VmCellSlot::Stack { .. } => None,
            })
            .collect();
        let id = frame.id;
        self.active.push(frame);
        (id, parked_set)
    }

    pub(super) fn exit_persistent_arena(
        &mut self,
        name: String,
        expected_id: u32,
        parked_set: &HashSet<usize>,
        strict: bool,
    ) -> Result<(), &'static str> {
        let mut frame = self.active.pop().expect("active persistent VM arena frame");
        debug_assert_eq!(frame.id, expected_id);
        let before = frame.cells.len();
        frame.cells.retain(|entry| match entry {
            VmCellSlot::Stack { .. } => false,
            VmCellSlot::Upval(w) => w.strong_count() > 0,
        });
        let freed = before - frame.cells.len();
        self.reusable_slots += freed;
        frame.live = frame.cells.len();
        let overflow = frame.budget.is_some_and(|budget| frame.live > budget);
        let leaked = strict
            && frame.cells.iter().any(|entry| match entry {
                VmCellSlot::Upval(w) => {
                    let ptr = Weak::as_ptr(w) as usize;
                    !parked_set.contains(&ptr) && !frame.blessed.contains(&ptr)
                }
                VmCellSlot::Stack { .. } => false,
            });
        frame.blessed.clear();
        self.persistent.push((name, frame));
        if overflow {
            return Err("Region cell budget exceeded");
        }
        if leaked {
            return Err("Strict arena retained new cells; drop them before the run ends");
        }
        Ok(())
    }

    pub(super) fn reset_persistent(&mut self, name: &str) {
        if let Some(index) = self.persistent.iter().position(|(n, _)| n == name) {
            let (_, frame) = self.persistent.swap_remove(index);
            let mut freed = 0usize;
            for entry in frame.cells {
                if let VmCellSlot::Upval(w) = entry
                    && let Some(upval) = w.upgrade()
                {
                    *upval.borrow_mut() = Upvalue::Closed(Value::Null);
                    freed += 1;
                }
            }
            self.reusable_slots += freed;
        }
    }

    pub(super) fn arena_stats_record(&mut self, name: &str) -> RecordMap<'static> {
        for frame in self.active.iter_mut() {
            if frame.persistent && self.stats[frame.stats].name.as_deref() == Some(name) {
                let before = frame.cells.len();
                frame.cells.retain(|entry| match entry {
                    VmCellSlot::Stack { .. } => true,
                    VmCellSlot::Upval(w) => w.strong_count() > 0,
                });
                let freed = before - frame.cells.len();
                self.reusable_slots += freed;
                frame.live = frame.cells.len();
            }
        }
        for (parked_name, frame) in self.persistent.iter_mut() {
            if parked_name == name {
                let before = frame.cells.len();
                frame.cells.retain(|entry| match entry {
                    VmCellSlot::Stack { .. } => false,
                    VmCellSlot::Upval(w) => w.strong_count() > 0,
                });
                let freed = before - frame.cells.len();
                self.reusable_slots += freed;
                frame.live = frame.cells.len();
            }
        }
        let stats = self
            .stats
            .iter()
            .find(|entry| entry.name.as_deref() == Some(name));
        let metric = |pick: fn(&RegionStats) -> u64| stats.map_or(0.0, |entry| pick(entry) as f64);
        let live = self
            .active
            .iter()
            .rev()
            .find(|frame| frame.persistent && self.stats[frame.stats].name.as_deref() == Some(name))
            .or_else(|| {
                self.persistent
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, frame)| frame)
            })
            .map_or(0.0, |frame| frame.live as f64);
        RecordMap::from([
            ("name".into(), Value::String(name.to_owned())),
            ("entries".into(), Value::Number(metric(|s| s.entries))),
            ("allocated".into(), Value::Number(metric(|s| s.allocated))),
            (
                "reused_slots".into(),
                Value::Number(metric(|s| s.reused_slots)),
            ),
            (
                "peak_live".into(),
                Value::Number(stats.map_or(0.0, |s| s.peak_live as f64)),
            ),
            ("promoted".into(), Value::Number(metric(|s| s.promoted))),
            (
                "suggested_budget".into(),
                Value::Number(stats.map_or(0.0, |s| s.suggested_budget() as f64)),
            ),
            ("live".into(), Value::Number(live)),
        ])
    }
}
