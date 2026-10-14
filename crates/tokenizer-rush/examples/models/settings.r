// Derived nodes may refer to bindings declared later.
node settings: Settings = Settings({size: scale(3)})
struct Settings { size: number }
fn scale(x: number) -> number { return x * width }
param width: number = 2
show settings.size
