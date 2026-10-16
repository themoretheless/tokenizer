use super::{Environment, Result, Runtime};
use crate::Expr;
use themoretheless_tokenizer_core::Span;

/// One active region (arena) frame.
#[derive(Clone, Debug)]
pub(super) struct RegionFrame {
    pub(super) id: u32,
    /// Live-cell ceiling; `None` for an unbounded region.
    pub(super) budget: Option<usize>,
    /// Cells currently alive with this region's stamp.
    pub(super) live: usize,
    /// Index into `Runtime::region_stats` for this region's name.
    pub(super) stats: usize,
    /// Cell slots stamped to this region, so exit sweeps only its own cells
    /// instead of scanning the whole table. May hold stale entries (freed or
    /// re-stamped slots); the exit pass re-checks the stamp and liveness.
    pub(super) cells: Vec<usize>,
}

/// Aggregated allocation metrics for one region name, or for all anonymous
/// regions under `name: None`. Counters accumulate across every entry into
/// the region, including inside coroutines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionStats {
    /// Region name as written in source; `None` aggregates anonymous blocks.
    pub name: Option<String>,
    /// How many times the region was entered.
    pub entries: u64,
    /// Total cell allocations stamped to the region.
    pub allocated: u64,
    /// Allocations that reused a previously freed cell slot.
    pub reused_slots: u64,
    /// Maximum simultaneously live cells observed; use it to size budgets.
    pub peak_live: usize,
    /// Cells promoted to the parent region because they were still reachable
    /// at region exit.
    pub promoted: u64,
}

impl RegionStats {
    /// Budget recommendation from the observed peak: the smallest power of
    /// two above `peak_live`, giving at least 2x headroom for load spikes.
    /// Treat it as a starting point, not a proof; rerun under realistic load.
    #[must_use]
    pub fn suggested_budget(&self) -> usize {
        (self.peak_live + 1).next_power_of_two()
    }
}

impl<'s> Runtime<'_, 's> {
    /// Parked coroutine regions: frames above `base` leave the shared stack
    /// while the task is suspended.
    pub(super) fn take_regions_above(&mut self, base: usize) -> Vec<RegionFrame> {
        self.region_stack.split_off(base)
    }

    /// A resumed coroutine puts its parked region frames back on top.
    pub(super) fn restore_regions(&mut self, frames: Vec<RegionFrame>) {
        self.region_stack.extend(frames);
    }

    /// Drop region frames of a terminated coroutine without sweeping; their
    /// cells are reclaimed through the normal release path.
    pub(super) fn truncate_regions(&mut self, base: usize) {
        self.region_stack.truncate(base);
    }

    /// Evaluate an optional region budget expression in the parent region.
    pub(super) fn eval_region_budget(
        &mut self,
        budget: Option<&Expr<'s>>,
        environment: &Environment<'s>,
    ) -> Result<Option<usize>> {
        let Some(expression) = budget else {
            return Ok(None);
        };
        if let Some(budget) = self.expr(expression, environment)?.as_usize_index() {
            Ok(Some(budget))
        } else {
            self.error(
                expression.span,
                "Region budget must be a non-negative integer",
            )
        }
    }

    /// Open a region (arena) frame; cells allocated inside get its stamp and
    /// count towards the metrics of its name.
    pub(super) fn enter_region(&mut self, budget: Option<usize>, name: Option<&str>) -> u32 {
        self.next_region = self.next_region.checked_add(1).expect("region id overflow");
        let region = self.next_region;
        let stats = match self
            .region_stats
            .iter()
            .position(|entry| entry.name.as_deref() == name)
        {
            Some(index) => index,
            None => {
                let index = self.region_stats.len();
                self.region_stats.push(RegionStats {
                    name: name.map(str::to_owned),
                    ..RegionStats::default()
                });
                index
            }
        };
        self.region_stats[stats].entries += 1;
        self.region_stack.push(RegionFrame {
            id: region,
            budget,
            live: 0,
            stats,
            cells: Vec::new(),
        });
        region
    }

    /// Close a region frame: cells no longer referenced after scope release
    /// are swept in bulk; escaping values promote to the parent region and
    /// count against its budget.
    pub(super) fn exit_region(&mut self, region: u32, span: Span) -> Result<()> {
        let frame = self.region_stack.pop().expect("region frame");
        debug_assert_eq!(frame.id, region);
        self.reclaim_cells();
        let parent = self.region_stack.last().map_or(0, |frame| frame.id);
        let mut promoted = 0_usize;
        for index in frame.cells {
            // Stale entries belong to freed or re-stamped slots.
            if self.cell_regions.get(index).copied() != Some(region) {
                continue;
            }
            if let Some(id) = self.cell_ids[index].upgrade() {
                self.cell_regions[index] = parent;
                promoted += 1;
                if let Some(parent_frame) = self.region_stack.last_mut() {
                    parent_frame.cells.push(index);
                }
                drop(id);
            }
        }
        self.region_stats[frame.stats].promoted += promoted as u64;
        if promoted > 0
            && let Some(parent_frame) = self.region_stack.last_mut()
        {
            parent_frame.live += promoted;
            if parent_frame
                .budget
                .is_some_and(|budget| parent_frame.live > budget)
            {
                return self.error(span, "Region cell budget exceeded");
            }
        }
        Ok(())
    }
}
