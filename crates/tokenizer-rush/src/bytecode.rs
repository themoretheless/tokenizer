//! Flat-frame Bytecode VM and AST-to-bytecode compiler for Rush.
//!
//! Provides high-performance, flat stack-frame execution of compiled Rush code,
//! bypassing recursive AST walking and Environment map cloning overhead.
//! Supports first-class user functions (`fn`), anonymous lambdas, closures with
//! captured lexical upvalues (mutable and immutable), and higher-order builtins.

pub mod compiler;
pub mod iterator;
pub mod opcode;
pub mod vm;

use crate::bytecode::compiler::Compiler;
use crate::{CancellationToken, ExecutionLimits, RuntimeError, Value};

pub use iterator::{
    BytecodeIterator, BytecodeSequence, BytecodeSequenceSource, BytecodeSequenceStage,
    BytecodeStageKind,
};
pub use opcode::{BytecodeClosure, BytecodeFunction, Chunk, Opcode, Upvalue, UpvalueDescriptor};
pub use vm::Vm;

/// Compiled bytecode program ready for execution.
#[derive(Clone, Debug, PartialEq)]
pub struct BytecodeProgram {
    pub chunk: Chunk,
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
        let mut compiler = Compiler::new();
        compiler.compile_module(&parsed.module.items)?;
        let top_fn = compiler.pop_state();
        Ok(Self {
            chunk: top_fn.chunk,
        })
    }

    /// Execute the compiled bytecode program with a step budget limit.
    pub fn execute(&self, budget: usize) -> Result<Value<'static>, RuntimeError> {
        let mut vm = Vm::new(budget);
        vm.run(&self.chunk)
    }

    /// Execute the compiled bytecode program with explicit `ExecutionLimits` and optional `CancellationToken`.
    pub fn execute_with_limits(
        &self,
        limits: ExecutionLimits,
        cancel: Option<&CancellationToken>,
    ) -> Result<Value<'static>, RuntimeError> {
        let mut vm = Vm::with_limits(limits, cancel);
        vm.run(&self.chunk)
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
