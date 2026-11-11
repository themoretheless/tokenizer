//! Consolidated table-driven syntax engines for 49 programming and query languages.
//!
//! Each language module is feature-gated by its canonical [`LanguageId`](themoretheless_tokenizer_core::LanguageId)
//! and backed by [`FullEngine`](themoretheless_tokenizer_core::FullEngine) + compile-time
//! [`KeywordLookup`](themoretheless_tokenizer_core::KeywordLookup).

#![forbid(unsafe_code)]

mod domain;
mod functional;
mod managed;
mod scripting;
mod systems;

#[cfg(feature = "abap")]
pub use domain::abap;
#[cfg(feature = "graphql")]
pub use domain::graphql;
#[cfg(feature = "mongo")]
pub use domain::mongo;
#[cfg(feature = "solidity")]
pub use domain::solidity;
#[cfg(feature = "sql")]
pub use domain::sql;
#[cfg(feature = "verilog")]
pub use domain::verilog;
#[cfg(feature = "vhdl")]
pub use domain::vhdl;
#[cfg(feature = "clojure")]
pub use functional::clojure;
#[cfg(feature = "elixir")]
pub use functional::elixir;
#[cfg(feature = "erlang")]
pub use functional::erlang;
#[cfg(feature = "fsharp")]
pub use functional::fsharp;
#[cfg(feature = "haskell")]
pub use functional::haskell;
#[cfg(feature = "lisp")]
pub use functional::lisp;
#[cfg(feature = "ocaml")]
pub use functional::ocaml;
#[cfg(feature = "prolog")]
pub use functional::prolog;
#[cfg(feature = "scheme")]
pub use functional::scheme;
#[cfg(feature = "csharp")]
pub use managed::csharp;
#[cfg(feature = "dart")]
pub use managed::dart;
#[cfg(feature = "delphi")]
pub use managed::delphi;
#[cfg(feature = "groovy")]
pub use managed::groovy;
#[cfg(feature = "java")]
pub use managed::java;
#[cfg(feature = "kotlin")]
pub use managed::kotlin;
#[cfg(feature = "scala")]
pub use managed::scala;
#[cfg(feature = "swift")]
pub use managed::swift;
#[cfg(feature = "visualbasic")]
pub use managed::visualbasic;
#[cfg(feature = "bash")]
pub use scripting::bash;
#[cfg(feature = "javascript")]
pub use scripting::javascript;
#[cfg(feature = "julia")]
pub use scripting::julia;
#[cfg(feature = "lua")]
pub use scripting::lua;
#[cfg(feature = "matlab")]
pub use scripting::matlab;
#[cfg(feature = "perl")]
pub use scripting::perl;
#[cfg(feature = "php")]
pub use scripting::php;
#[cfg(feature = "powershell")]
pub use scripting::powershell;
#[cfg(feature = "python")]
pub use scripting::python;
#[cfg(feature = "r")]
pub use scripting::r;
#[cfg(feature = "ruby")]
pub use scripting::ruby;
#[cfg(feature = "typescript")]
pub use scripting::typescript;
#[cfg(feature = "ada")]
pub use systems::ada;
#[cfg(feature = "assembly")]
pub use systems::assembly;
#[cfg(feature = "c")]
pub use systems::c;
#[cfg(feature = "cobol")]
pub use systems::cobol;
#[cfg(feature = "cpp")]
pub use systems::cpp;
#[cfg(feature = "dlang")]
pub use systems::dlang;
#[cfg(feature = "fortran")]
pub use systems::fortran;
#[cfg(feature = "go")]
pub use systems::go;
#[cfg(feature = "nim")]
pub use systems::nim;
#[cfg(feature = "objectivec")]
pub use systems::objectivec;
#[cfg(feature = "rust")]
pub use systems::rust;
#[cfg(feature = "zig")]
pub use systems::zig;

/// Register all Cargo-feature-enabled profiled languages into `builder`.
pub fn register_enabled(
    builder: &mut themoretheless_tokenizer_core::RegistryBuilder,
) -> Result<(), themoretheless_tokenizer_core::RegisterError> {
    #[cfg(feature = "abap")]
    {
        builder.register(&abap::ENGINE)?;
    }
    #[cfg(feature = "ada")]
    {
        builder.register(&ada::ENGINE)?;
    }
    #[cfg(feature = "assembly")]
    {
        builder.register(&assembly::ENGINE)?;
    }
    #[cfg(feature = "bash")]
    {
        builder.register(&bash::ENGINE)?;
    }
    #[cfg(feature = "c")]
    {
        builder.register(&c::ENGINE)?;
    }
    #[cfg(feature = "clojure")]
    {
        builder.register(&clojure::ENGINE)?;
    }
    #[cfg(feature = "cobol")]
    {
        builder.register(&cobol::ENGINE)?;
    }
    #[cfg(feature = "cpp")]
    {
        builder.register(&cpp::ENGINE)?;
    }
    #[cfg(feature = "csharp")]
    {
        builder.register(&csharp::ENGINE)?;
    }
    #[cfg(feature = "dart")]
    {
        builder.register(&dart::ENGINE)?;
    }
    #[cfg(feature = "delphi")]
    {
        builder.register(&delphi::ENGINE)?;
    }
    #[cfg(feature = "dlang")]
    {
        builder.register(&dlang::ENGINE)?;
    }
    #[cfg(feature = "elixir")]
    {
        builder.register(&elixir::ENGINE)?;
    }
    #[cfg(feature = "erlang")]
    {
        builder.register(&erlang::ENGINE)?;
    }
    #[cfg(feature = "fortran")]
    {
        builder.register(&fortran::ENGINE)?;
    }
    #[cfg(feature = "fsharp")]
    {
        builder.register(&fsharp::ENGINE)?;
    }
    #[cfg(feature = "go")]
    {
        builder.register(&go::ENGINE)?;
    }
    #[cfg(feature = "graphql")]
    {
        builder.register(&graphql::ENGINE)?;
    }
    #[cfg(feature = "groovy")]
    {
        builder.register(&groovy::ENGINE)?;
    }
    #[cfg(feature = "haskell")]
    {
        builder.register(&haskell::ENGINE)?;
    }
    #[cfg(feature = "java")]
    {
        builder.register(&java::ENGINE)?;
    }
    #[cfg(feature = "javascript")]
    {
        builder.register(&javascript::ENGINE)?;
    }
    #[cfg(feature = "julia")]
    {
        builder.register(&julia::ENGINE)?;
    }
    #[cfg(feature = "kotlin")]
    {
        builder.register(&kotlin::ENGINE)?;
    }
    #[cfg(feature = "lisp")]
    {
        builder.register(&lisp::ENGINE)?;
    }
    #[cfg(feature = "lua")]
    {
        builder.register(&lua::ENGINE)?;
    }
    #[cfg(feature = "matlab")]
    {
        builder.register(&matlab::ENGINE)?;
    }
    #[cfg(feature = "mongo")]
    {
        builder.register(&mongo::ENGINE)?;
    }
    #[cfg(feature = "nim")]
    {
        builder.register(&nim::ENGINE)?;
    }
    #[cfg(feature = "objectivec")]
    {
        builder.register(&objectivec::ENGINE)?;
    }
    #[cfg(feature = "ocaml")]
    {
        builder.register(&ocaml::ENGINE)?;
    }
    #[cfg(feature = "perl")]
    {
        builder.register(&perl::ENGINE)?;
    }
    #[cfg(feature = "php")]
    {
        builder.register(&php::ENGINE)?;
    }
    #[cfg(feature = "powershell")]
    {
        builder.register(&powershell::ENGINE)?;
    }
    #[cfg(feature = "prolog")]
    {
        builder.register(&prolog::ENGINE)?;
    }
    #[cfg(feature = "python")]
    {
        builder.register(&python::ENGINE)?;
    }
    #[cfg(feature = "r")]
    {
        builder.register(&r::ENGINE)?;
    }
    #[cfg(feature = "ruby")]
    {
        builder.register(&ruby::ENGINE)?;
    }
    #[cfg(feature = "rust")]
    {
        builder.register(&rust::ENGINE)?;
    }
    #[cfg(feature = "scala")]
    {
        builder.register(&scala::ENGINE)?;
    }
    #[cfg(feature = "scheme")]
    {
        builder.register(&scheme::ENGINE)?;
    }
    #[cfg(feature = "solidity")]
    {
        builder.register(&solidity::ENGINE)?;
    }
    #[cfg(feature = "sql")]
    {
        builder.register(&sql::ENGINE)?;
    }
    #[cfg(feature = "swift")]
    {
        builder.register(&swift::ENGINE)?;
    }
    #[cfg(feature = "typescript")]
    {
        builder.register(&typescript::ENGINE)?;
    }
    #[cfg(feature = "verilog")]
    {
        builder.register(&verilog::ENGINE)?;
    }
    #[cfg(feature = "vhdl")]
    {
        builder.register(&vhdl::ENGINE)?;
    }
    #[cfg(feature = "visualbasic")]
    {
        builder.register(&visualbasic::ENGINE)?;
    }
    #[cfg(feature = "zig")]
    {
        builder.register(&zig::ENGINE)?;
    }
    let _ = builder;
    Ok(())
}
