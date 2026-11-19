//! Flat-frame Bytecode VM and AST-to-bytecode compiler for Rush.
//!
//! Provides high-performance, flat stack-frame execution of compiled Rush code,
//! bypassing recursive AST walking and Environment map cloning overhead.
//! Supports first-class user functions (`fn`), anonymous lambdas, closures with
//! captured lexical upvalues (mutable and immutable), and higher-order builtins.

pub mod compiler;
pub mod iterator;
pub mod opcode;
mod scalar_file;
pub mod vm;

use crate::bytecode::compiler::Compiler;
use crate::{CancellationToken, ExecutionLimits, RuntimeError, Value};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

pub use iterator::{
    BytecodeIterator, BytecodeSequence, BytecodeSequenceSource, BytecodeSequenceStage,
    BytecodeStageKind,
};
pub use opcode::{BytecodeClosure, BytecodeFunction, Chunk, Opcode, Upvalue, UpvalueDescriptor};
pub use vm::Vm;

/// Compiled bytecode program ready for execution.
#[derive(Clone, Debug)]
pub struct BytecodeProgram {
    entry: Rc<BytecodeClosure>,
}

impl From<Chunk> for BytecodeProgram {
    fn from(chunk: Chunk) -> Self {
        Self {
            entry: Rc::new(BytecodeClosure {
                function: Rc::new(BytecodeFunction {
                    name: Some("<main>".into()),
                    arity: 0,
                    chunk,
                    upvalue_descriptors: Vec::new(),
                }),
                upvalues: Vec::new(),
            }),
        }
    }
}

impl Deref for BytecodeProgram {
    type Target = BytecodeFunction;

    fn deref(&self) -> &BytecodeFunction {
        &self.entry.function
    }
}

impl DerefMut for BytecodeProgram {
    fn deref_mut(&mut self) -> &mut BytecodeFunction {
        Rc::make_mut(&mut Rc::make_mut(&mut self.entry).function)
    }
}

impl PartialEq for BytecodeProgram {
    fn eq(&self, other: &Self) -> bool {
        self.chunk == other.chunk
    }
}

impl BytecodeProgram {
    /// Parse and compile Rush source code into a `BytecodeProgram`.
    pub fn compile(source: &str) -> Result<Self, RuntimeError> {
        let parsed = crate::parse(source);
        if let Some(diag) = parsed.diagnostics.first() {
            return Err(RuntimeError {
                module: None,
                span: diag.span,
                message: diag.message.to_string(),
                stack: Vec::new(),
                location: None,
            });
        }
        Self::compile_items(&parsed.module.items)
    }

    pub(crate) fn compile_items(items: &[crate::Stmt<'_>]) -> Result<Self, RuntimeError> {
        let mut compiler = Compiler::new();
        compiler.compile_module(items)?;
        Ok(Self::from(compiler.pop_state().chunk))
    }

    pub(crate) fn compile_scalar_file(
        items: &[crate::Stmt<'_>],
        contracts: &std::collections::HashMap<usize, crate::analysis::FunctionContract>,
    ) -> Option<Self> {
        if scalar_file::supports(items, contracts) {
            Self::compile_items(items).ok()
        } else {
            None
        }
    }

    /// Execute the compiled bytecode program with a step budget limit.
    pub fn execute(&self, budget: usize) -> Result<Value<'static>, RuntimeError> {
        let mut vm = Vm::new(budget);
        vm.run_closure(&self.entry, &[])
    }

    /// Execute the compiled bytecode program with explicit `ExecutionLimits` and optional `CancellationToken`.
    pub fn execute_with_limits(
        &self,
        limits: ExecutionLimits,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value<'static>, RuntimeError> {
        let mut vm = Vm::with_limits(limits, cancel);
        vm.run_closure(&self.entry, &[])
    }
}

/// Compile and evaluate source code directly via the Bytecode VM.
pub fn evaluate_bytecode(source: &str, budget: usize) -> Result<Value<'static>, RuntimeError> {
    let program = BytecodeProgram::compile(source)?;
    program.execute(budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_a_cloned_chunk_preserves_prepared_code() {
        let program = BytecodeProgram::compile("2 + 3").unwrap();
        let mut edited = program.clone();
        edited.chunk.constants[0] = Value::Number(7.0);
        assert_eq!(program.execute(100).unwrap(), Value::Number(5.0));
        assert_eq!(edited.execute(100).unwrap(), Value::Number(7.0));
    }

    #[test]
    fn bytecode_get_returns_option_and_supports_tuple() {
        let val = evaluate_bytecode("get([10, 20], 1)", 1000).unwrap();
        assert_eq!(val, Value::Variant("Some", vec![Value::Number(20.0)]));

        let none_val = evaluate_bytecode("get([10, 20], 5)", 1000).unwrap();
        assert_eq!(none_val, Value::Variant("None", vec![]));

        let tuple_val = evaluate_bytecode("get((100, 200), 0)", 1000).unwrap();
        assert_eq!(
            tuple_val,
            Value::Variant("Some", vec![Value::Number(100.0)])
        );

        let rec_val = evaluate_bytecode("get({ a: 42 }, \"a\")", 1000).unwrap();
        assert_eq!(rec_val, Value::Variant("Some", vec![Value::Number(42.0)]));
    }

    #[test]
    fn bytecode_len_counts_unicode_scalars() {
        let val = evaluate_bytecode("len(\"привет\")", 1000).unwrap();
        assert_eq!(val, Value::Number(6.0));
    }

    #[test]
    fn bytecode_pow_and_extended_math_builtins() {
        let val = evaluate_bytecode("clamp(2 ** 10, 0, 500) + lerp(0, 100, 0.25)", 1000).unwrap();
        assert_eq!(val, Value::Number(525.0));

        let dot_val = evaluate_bytecode("dot(vec3(1, 2, 3), vec3(4, 5, 6))", 1000).unwrap();
        assert_eq!(dot_val, Value::Number(32.0));
    }

    #[test]
    fn bytecode_cancellation_token_stops_execution() {
        let prog = BytecodeProgram::compile("let mut i = 0;\nwhile true { i += 1; }\ni").unwrap();
        let token = CancellationToken::default();
        token.cancel();
        let err = prog
            .execute_with_limits(ExecutionLimits::new(1_000_000), Some(&token))
            .unwrap_err();
        assert!(err.message.contains("cancelled"));
    }
}
