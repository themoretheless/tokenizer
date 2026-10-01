use mlua::Lua;
fn main() {
    let lua = Lua::new();
    let program = lua
        .load("local values = {}\nfor i=1,1000000 do values[i] = {i,i,i,i} end\nreturn #values")
        .into_function()
        .unwrap();
    lua.gc_collect().unwrap();
    let baseline = lua.used_memory();
    let limit = baseline + 256 * 1024;
    assert_eq!(lua.set_memory_limit(limit).unwrap(), 0);
    let error = program.call::<usize>(()).unwrap_err();
    assert!(matches!(error, mlua::Error::MemoryError(_)), "{error:?}");
    let after_failure = lua.used_memory();
    assert!(
        after_failure <= limit,
        "VM allocations exceeded configured limit"
    );
    // Temporarily remove the cap for host-directed cleanup, then restore it.
    assert_eq!(lua.set_memory_limit(0).unwrap(), limit);
    drop(program);
    lua.gc_collect().unwrap();
    lua.gc_collect().unwrap();
    let after_cleanup = lua.used_memory();
    assert!(after_cleanup < after_failure);
    lua.set_memory_limit(limit).unwrap();
    assert_eq!(lua.load("return 1+2").eval::<i64>().unwrap(), 3);
    assert!(lua.used_memory() <= limit);
    println!(
        "{}; allocator memory bytes, not RSS or host allocations",
        env!("CARGO_PKG_NAME")
    );
    println!("baseline\tlimit\tafter_failure\tafter_cleanup\treuse_result");
    println!("{baseline}\t{limit}\t{after_failure}\t{after_cleanup}\t3");
}
