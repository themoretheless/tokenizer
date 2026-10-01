#!/bin/sh
set -eu
# Pass an extracted official Wren 0.4.0 source directory and output executable.
if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
  echo 'Usage: build.sh WREN_SOURCE_DIR OUTPUT_EXECUTABLE [runtime|errors|host-object|startup|memory-limit|cancellation]' >&2
  exit 2
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
case "${3:-runtime}" in
  runtime) source_file=main.c ;;
  errors) source_file=errors.c ;;
  host-object) source_file=host-object.c ;;
  startup) source_file=startup.c ;;
  memory-limit) source_file=memory-limit.c ;;
  cancellation) source_file=cancellation.c ;;
  *) echo 'Unknown Wren adapter' >&2; exit 2 ;;
esac
cc -O3 -DNDEBUG -DWREN_OPT_META=0 -DWREN_OPT_RANDOM=0 \
  -I"$1/src/include" -I"$1/src/vm" \
  "$script_dir/$source_file" "$1"/src/vm/*.c -lm -o "$2"
