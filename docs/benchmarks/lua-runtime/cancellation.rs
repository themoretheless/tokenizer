include!("../lua-cancellation-shared.rs");
fn main() {
    run(
        |lua, flag| {
            lua.set_hook(
                mlua::HookTriggers::new().every_nth_instruction(1000),
                move |_, _| check(&flag),
            )
            .unwrap();
        },
        Lua::remove_hook,
    );
}
