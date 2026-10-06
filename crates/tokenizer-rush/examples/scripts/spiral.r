// Run with turns=3 scale=1. Generates points for a host to consume.
range(0, 360 * turns, 5) | map(a => vec2(cos(deg(a)), sin(deg(a))) * (a / 360 * scale))
