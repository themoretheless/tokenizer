include!("../lua-cancellation-shared.rs");
fn main() {
    run(
        |lua, flag| lua.set_interrupt(move |_| check(&flag)),
        Lua::remove_interrupt,
    );
}
