//! Deterministic traversal of the borrowing JSON AST.

use super::{Array, Member, Object, Parse, Value};

/// Controls traversal after a visitor hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VisitControl {
    /// Continue with children and later siblings.
    Continue,
    /// Do not visit children of the current value, member, or array element.
    ///
    /// The matching leave hook is still called. From a leave hook this has the
    /// same effect as [`Continue`](Self::Continue).
    SkipChildren,
    /// Stop immediately. Matching leave hooks are not called while unwinding.
    Break,
}

/// Result of a complete or interrupted traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VisitOutcome {
    Completed,
    Broken,
}

impl VisitOutcome {
    #[must_use]
    pub const fn is_completed(self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// The parent relationship through which a value is visited.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum VisitContext<'ast, 'source> {
    Root,
    ObjectMember {
        object: &'ast Object<'source>,
        member: &'ast Member<'source>,
        member_index: usize,
    },
    ArrayElement {
        array: &'ast Array<'source>,
        index: usize,
    },
}

/// Hooks for deterministic depth-first traversal of a JSON AST.
///
/// Values are visited in preorder by `enter_value` and in postorder by
/// `leave_value`. Object members retain source order and array elements retain
/// index order. Every hook defaults to [`VisitControl::Continue`].
///
/// ```
/// use themoretheless_tokenizer::json::{
///     AstVisitor, Value, VisitContext, VisitControl, parse, visit_parse,
/// };
///
/// #[derive(Default)]
/// struct Numbers(Vec<String>);
///
/// impl<'ast, 'source: 'ast> AstVisitor<'ast, 'source> for Numbers {
///     fn enter_value(
///         &mut self,
///         value: &'ast Value<'source>,
///         _context: VisitContext<'ast, 'source>,
///     ) -> VisitControl {
///         if let Some(number) = value.as_number() {
///             self.0.push(number.as_str().to_owned());
///         }
///         VisitControl::Continue
///     }
/// }
///
/// let parsed = parse(r#"{"values":[1,2]}"#);
/// let mut numbers = Numbers::default();
/// assert!(visit_parse(&parsed, &mut numbers).is_completed());
/// assert_eq!(numbers.0, ["1", "2"]);
/// ```
pub trait AstVisitor<'ast, 'source: 'ast> {
    fn enter_value(
        &mut self,
        _value: &'ast Value<'source>,
        _context: VisitContext<'ast, 'source>,
    ) -> VisitControl {
        VisitControl::Continue
    }

    fn leave_value(
        &mut self,
        _value: &'ast Value<'source>,
        _context: VisitContext<'ast, 'source>,
    ) -> VisitControl {
        VisitControl::Continue
    }

    fn enter_object_member(
        &mut self,
        _object: &'ast Object<'source>,
        _member: &'ast Member<'source>,
        _member_index: usize,
    ) -> VisitControl {
        VisitControl::Continue
    }

    fn leave_object_member(
        &mut self,
        _object: &'ast Object<'source>,
        _member: &'ast Member<'source>,
        _member_index: usize,
    ) -> VisitControl {
        VisitControl::Continue
    }

    fn enter_array_element(
        &mut self,
        _array: &'ast Array<'source>,
        _element: &'ast Value<'source>,
        _index: usize,
    ) -> VisitControl {
        VisitControl::Continue
    }

    fn leave_array_element(
        &mut self,
        _array: &'ast Array<'source>,
        _element: &'ast Value<'source>,
        _index: usize,
    ) -> VisitControl {
        VisitControl::Continue
    }
}

/// Visits a root value and all descendants in deterministic depth-first order.
#[must_use]
pub fn visit_value<'ast, 'source: 'ast, V>(
    root: &'ast Value<'source>,
    visitor: &mut V,
) -> VisitOutcome
where
    V: AstVisitor<'ast, 'source> + ?Sized,
{
    walk_value(root, VisitContext::Root, visitor)
}

/// Visits the recovered root of a parse, if one exists.
///
/// A rootless parse is a successful no-op. Diagnostics do not prevent
/// traversal of values retained by parser recovery.
#[must_use]
pub fn visit_parse<'ast, 'source: 'ast, V>(
    parsed: &'ast Parse<'source>,
    visitor: &mut V,
) -> VisitOutcome
where
    V: AstVisitor<'ast, 'source> + ?Sized,
{
    parsed
        .value()
        .map_or(VisitOutcome::Completed, |root| visit_value(root, visitor))
}

fn walk_value<'ast, 'source: 'ast, V>(
    value: &'ast Value<'source>,
    context: VisitContext<'ast, 'source>,
    visitor: &mut V,
) -> VisitOutcome
where
    V: AstVisitor<'ast, 'source> + ?Sized,
{
    match visitor.enter_value(value, context) {
        VisitControl::Break => return VisitOutcome::Broken,
        VisitControl::SkipChildren => return finish_value(value, context, visitor),
        VisitControl::Continue => {}
    }

    let child_outcome = match value {
        Value::Object(object) => walk_object(object, visitor),
        Value::Array(array) => walk_array(array, visitor),
        Value::String(_) | Value::Number(_) | Value::Boolean(_) | Value::Null(_) => {
            VisitOutcome::Completed
        }
    };
    if child_outcome == VisitOutcome::Broken {
        return child_outcome;
    }
    finish_value(value, context, visitor)
}

fn walk_object<'ast, 'source: 'ast, V>(
    object: &'ast Object<'source>,
    visitor: &mut V,
) -> VisitOutcome
where
    V: AstVisitor<'ast, 'source> + ?Sized,
{
    for (member_index, member) in object.members().iter().enumerate() {
        match visitor.enter_object_member(object, member, member_index) {
            VisitControl::Break => return VisitOutcome::Broken,
            VisitControl::SkipChildren => {}
            VisitControl::Continue => {
                let context = VisitContext::ObjectMember {
                    object,
                    member,
                    member_index,
                };
                if walk_value(member.value(), context, visitor) == VisitOutcome::Broken {
                    return VisitOutcome::Broken;
                }
            }
        }
        if visitor.leave_object_member(object, member, member_index) == VisitControl::Break {
            return VisitOutcome::Broken;
        }
    }
    VisitOutcome::Completed
}

fn walk_array<'ast, 'source: 'ast, V>(array: &'ast Array<'source>, visitor: &mut V) -> VisitOutcome
where
    V: AstVisitor<'ast, 'source> + ?Sized,
{
    for (index, element) in array.elements().iter().enumerate() {
        match visitor.enter_array_element(array, element, index) {
            VisitControl::Break => return VisitOutcome::Broken,
            VisitControl::SkipChildren => {}
            VisitControl::Continue => {
                let context = VisitContext::ArrayElement { array, index };
                if walk_value(element, context, visitor) == VisitOutcome::Broken {
                    return VisitOutcome::Broken;
                }
            }
        }
        if visitor.leave_array_element(array, element, index) == VisitControl::Break {
            return VisitOutcome::Broken;
        }
    }
    VisitOutcome::Completed
}

fn finish_value<'ast, 'source: 'ast, V>(
    value: &'ast Value<'source>,
    context: VisitContext<'ast, 'source>,
    visitor: &mut V,
) -> VisitOutcome
where
    V: AstVisitor<'ast, 'source> + ?Sized,
{
    if visitor.leave_value(value, context) == VisitControl::Break {
        VisitOutcome::Broken
    } else {
        VisitOutcome::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{ValueKind, parse};

    #[derive(Default)]
    struct EventVisitor {
        events: Vec<String>,
    }

    impl<'ast, 'source: 'ast> AstVisitor<'ast, 'source> for EventVisitor {
        fn enter_value(
            &mut self,
            value: &'ast Value<'source>,
            context: VisitContext<'ast, 'source>,
        ) -> VisitControl {
            let context = match context {
                VisitContext::Root => "root".to_owned(),
                VisitContext::ObjectMember { member_index, .. } => format!("m{member_index}"),
                VisitContext::ArrayElement { index, .. } => format!("e{index}"),
            };
            self.events.push(format!("+{:?}:{context}", value.kind()));
            VisitControl::Continue
        }

        fn leave_value(
            &mut self,
            value: &'ast Value<'source>,
            _context: VisitContext<'ast, 'source>,
        ) -> VisitControl {
            self.events.push(format!("-{:?}", value.kind()));
            VisitControl::Continue
        }

        fn enter_object_member(
            &mut self,
            _object: &'ast Object<'source>,
            member: &'ast Member<'source>,
            member_index: usize,
        ) -> VisitControl {
            self.events.push(format!(
                "+member{member_index}:{}",
                member.key().decoded().unwrap()
            ));
            VisitControl::Continue
        }

        fn leave_object_member(
            &mut self,
            _object: &'ast Object<'source>,
            _member: &'ast Member<'source>,
            member_index: usize,
        ) -> VisitControl {
            self.events.push(format!("-member{member_index}"));
            VisitControl::Continue
        }

        fn enter_array_element(
            &mut self,
            _array: &'ast Array<'source>,
            _element: &'ast Value<'source>,
            index: usize,
        ) -> VisitControl {
            self.events.push(format!("+element{index}"));
            VisitControl::Continue
        }

        fn leave_array_element(
            &mut self,
            _array: &'ast Array<'source>,
            _element: &'ast Value<'source>,
            index: usize,
        ) -> VisitControl {
            self.events.push(format!("-element{index}"));
            VisitControl::Continue
        }
    }

    #[test]
    fn visits_in_deterministic_preorder_and_postorder() {
        let parsed = parse(r#"{"a":[1,true]}"#);
        let mut visitor = EventVisitor::default();
        assert_eq!(visit_parse(&parsed, &mut visitor), VisitOutcome::Completed);
        assert_eq!(
            visitor.events,
            [
                "+Object:root",
                "+member0:a",
                "+Array:m0",
                "+element0",
                "+Number:e0",
                "-Number",
                "-element0",
                "+element1",
                "+Boolean:e1",
                "-Boolean",
                "-element1",
                "-Array",
                "-member0",
                "-Object",
            ]
        );
    }

    #[test]
    fn skip_children_keeps_matching_leave_hook() {
        struct SkipFirst {
            entered_values: usize,
            left_members: usize,
        }
        impl<'ast, 'source: 'ast> AstVisitor<'ast, 'source> for SkipFirst {
            fn enter_value(
                &mut self,
                _value: &'ast Value<'source>,
                _context: VisitContext<'ast, 'source>,
            ) -> VisitControl {
                self.entered_values += 1;
                VisitControl::Continue
            }

            fn enter_object_member(
                &mut self,
                _object: &'ast Object<'source>,
                _member: &'ast Member<'source>,
                index: usize,
            ) -> VisitControl {
                if index == 0 {
                    VisitControl::SkipChildren
                } else {
                    VisitControl::Continue
                }
            }

            fn leave_object_member(
                &mut self,
                _object: &'ast Object<'source>,
                _member: &'ast Member<'source>,
                _index: usize,
            ) -> VisitControl {
                self.left_members += 1;
                VisitControl::Continue
            }
        }

        let parsed = parse(r#"{"skip":[1,2],"keep":3}"#);
        let mut visitor = SkipFirst {
            entered_values: 0,
            left_members: 0,
        };
        assert_eq!(visit_parse(&parsed, &mut visitor), VisitOutcome::Completed);
        assert_eq!(visitor.entered_values, 2);
        assert_eq!(visitor.left_members, 2);
    }

    #[test]
    fn break_stops_without_unwinding_leave_hooks() {
        struct BreakAtSecond {
            events: Vec<&'static str>,
        }
        impl<'ast, 'source: 'ast> AstVisitor<'ast, 'source> for BreakAtSecond {
            fn enter_array_element(
                &mut self,
                _array: &'ast Array<'source>,
                _element: &'ast Value<'source>,
                index: usize,
            ) -> VisitControl {
                self.events
                    .push(if index == 0 { "first" } else { "second" });
                if index == 1 {
                    VisitControl::Break
                } else {
                    VisitControl::Continue
                }
            }

            fn leave_value(
                &mut self,
                value: &'ast Value<'source>,
                _context: VisitContext<'ast, 'source>,
            ) -> VisitControl {
                if value.kind() == ValueKind::Array {
                    self.events.push("leave-array");
                }
                VisitControl::Continue
            }
        }

        let parsed = parse("[0,1,2]");
        let mut visitor = BreakAtSecond { events: Vec::new() };
        assert_eq!(visit_parse(&parsed, &mut visitor), VisitOutcome::Broken);
        assert_eq!(visitor.events, ["first", "second"]);
    }

    #[test]
    fn recovered_deep_and_rootless_parses_are_safe() {
        struct Counter(usize);
        impl<'ast, 'source: 'ast> AstVisitor<'ast, 'source> for Counter {
            fn enter_value(
                &mut self,
                _value: &'ast Value<'source>,
                _context: VisitContext<'ast, 'source>,
            ) -> VisitControl {
                self.0 += 1;
                VisitControl::Continue
            }
        }

        let deep = format!("{}01{}", "[".repeat(64), "]".repeat(64));
        let parsed = parse(&deep);
        assert!(!parsed.is_valid());
        let mut counter = Counter(0);
        assert_eq!(visit_parse(&parsed, &mut counter), VisitOutcome::Completed);
        assert_eq!(counter.0, 65);

        let rootless = parse("@");
        let mut counter = Counter(0);
        assert_eq!(
            visit_parse(&rootless, &mut counter),
            VisitOutcome::Completed
        );
        assert_eq!(counter.0, 0);
    }

    #[test]
    fn visitor_can_retain_borrowed_nodes_and_is_object_safe() {
        struct Collector<'ast, 'source> {
            values: Vec<&'ast Value<'source>>,
        }

        impl<'ast, 'source: 'ast> AstVisitor<'ast, 'source> for Collector<'ast, 'source> {
            fn enter_value(
                &mut self,
                value: &'ast Value<'source>,
                _context: VisitContext<'ast, 'source>,
            ) -> VisitControl {
                self.values.push(value);
                VisitControl::Continue
            }
        }

        fn visit_dynamically<'ast, 'source: 'ast>(
            root: &'ast Value<'source>,
            visitor: &mut dyn AstVisitor<'ast, 'source>,
        ) -> VisitOutcome {
            visit_value(root, visitor)
        }

        let parsed = parse(r#"{"escaped\nkey":[1,true]}"#);
        let root = parsed.value().unwrap();
        let mut collector = Collector { values: Vec::new() };
        assert_eq!(
            visit_dynamically(root, &mut collector),
            VisitOutcome::Completed
        );
        assert_eq!(collector.values.len(), 4);
        assert_eq!(collector.values[2].as_number().unwrap().as_i64(), Ok(1));
        assert_eq!(collector.values[3].as_bool(), Some(true));
    }
}
