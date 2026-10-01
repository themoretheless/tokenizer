// Each instance owns its position and velocity between frames.
mut position = vec3(0, 0, 0);
mut velocity = vec3(2, 0, -1);

fn update(delta: number) -> vec3 {
    position = position + velocity * delta;
    return publish_position(position);
}

fn set_velocity(next: vec3) {
    velocity = next;
}
