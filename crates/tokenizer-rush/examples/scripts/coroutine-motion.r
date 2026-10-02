// The scheduler interprets requests; the host advances time and emits events.
enum Wait { After(number), Event(str) }
mut position = 0
fn motion() -> number {
    let delay = 0.25
    for frame in range_iter(0, 3) {
        yield Wait.After(delay)
        position += 1
    }
    yield Wait.Event('finish')
    return position
}
