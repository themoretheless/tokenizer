#!/usr/bin/env -S rush --quiet
import sys

fn main() -> Result[bool, str] {
    let input = sys.input()?
    let output = sys.pipeline([['cat'], ['tr', 'a-z', 'A-Z']], input)?
    assert(output.code == 0, output.stderr)
    sys.out(output.stdout)?
    return Ok(true)
}

main()
