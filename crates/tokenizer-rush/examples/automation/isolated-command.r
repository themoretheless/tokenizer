#!/usr/bin/env -S rush --quiet
import sys

fn main() -> Result[bool, str] {
    sys.set_env('RUSH_MODE', 'host')?
    let options: sys.CommandOptions = sys.CommandOptions({
        cwd: Some('.'),
        env: [('RUSH_MODE', 'local')]
    })
    let output = sys.exec_with('sh', ['-c', 'printf "%s\n" "$RUSH_MODE"'], '', options)?
    assert(output.code == 0, output.stderr)
    assert(sys.env('RUSH_MODE') == Some('host'))
    sys.out(output.stdout)?
    return Ok(true)
}

main()
