#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "wren.h"
#if WREN_VERSION_NUMBER != 4000
#error "Requires Wren 0.4.0"
#endif

typedef union { max_align_t alignment; size_t size; } Header;
typedef struct { size_t live, peak, limit; } Allocator;

static void* allocate(void* memory, size_t size, void* data) {
  Allocator* state = data;
  Header* old = memory ? (Header*)memory - 1 : NULL;
  size_t previous = old ? old->size : 0;
  if (!size) {
    state->live -= previous;
    free(old);
    return NULL;
  }
  size_t retained = state->live - previous;
  if (size > SIZE_MAX - sizeof(Header) || size > SIZE_MAX - retained ||
      (state->limit && (retained > state->limit || size > state->limit - retained))) {
    fprintf(stderr, "allocator_denied live=%zu request=%zu previous=%zu limit=%zu\n",
            state->live, size, previous, state->limit);
    fflush(stderr);
    return NULL;
  }
  Header* next = realloc(old, sizeof(Header) + size);
  if (!next) { fputs("system_allocator_failed\n", stderr); exit(70); }
  next->size = size;
  state->live = retained + size;
  if (state->live > state->peak) state->peak = state->live;
  return next + 1;
}

int main(int argc, char** argv) {
  if (argc != 2 || (strcmp(argv[1], "unlimited") && strcmp(argv[1], "capped"))) return 2;
  Allocator state = {0};
  WrenConfiguration config;
  wrenInitConfiguration(&config);
  config.reallocateFn = allocate;
  config.userData = &state;
  WrenVM* vm = wrenNewVM(&config);
  const char* source = "class Work {\nstatic run() {\nvar values = []\n"
    "for (i in 0...10000) values.add([i,i,i,i])\nreturn values.count\n}\n}\n";
  if (wrenInterpret(vm, "memory", source) != WREN_RESULT_SUCCESS) return 3;
  wrenEnsureSlots(vm, 1);
  wrenGetVariable(vm, "memory", "Work", 0);
  WrenHandle* call = wrenMakeCallHandle(vm, "run()");
  wrenCollectGarbage(vm);
  size_t baseline = state.live;
  if (!strcmp(argv[1], "capped")) state.limit = baseline + 256 * 1024;
  WrenInterpretResult result = wrenCall(vm, call);
  fprintf(stderr, "call_returned result=%d\n", result);
  if (result != WREN_RESULT_SUCCESS || wrenGetSlotDouble(vm, 0) != 10000) return 4;
  wrenReleaseHandle(vm, call);
  wrenFreeVM(vm);
  printf("baseline=%zu peak=%zu after_free=%zu result=10000\n", baseline, state.peak, state.live);
  return state.live ? 5 : 0;
}
