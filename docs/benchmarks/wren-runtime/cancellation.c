#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <string.h>
#include <time.h>
#include "wren.h"
#if WREN_VERSION_NUMBER != 4000
#error "Requires Wren 0.4.0"
#endif
static int blocking;
static void entered(WrenVM* vm) {
  (void)vm;
  puts("entered");
  fflush(stdout);
  if (blocking) {
    struct timespec remaining = {0, 25000000};
    while (nanosleep(&remaining, &remaining)) {}
    puts("host_returned");
    fflush(stdout);
  }
}
static WrenForeignMethodFn bind(WrenVM* vm, const char* module, const char* name,
                               bool is_static, const char* signature) {
  (void)vm;
  return !strcmp(module, "cancel") && !strcmp(name, "Host") && is_static &&
         !strcmp(signature, "entered()") ? entered : NULL;
}
int main(int argc, char** argv) {
  if (argc != 2) return 2;
  int probe = !strcmp(argv[1], "probe");
  blocking = !strcmp(argv[1], "blocking_host");
  if (!probe && !blocking && strcmp(argv[1], "loop")) return 2;
  WrenConfiguration config;
  wrenInitConfiguration(&config);
  config.bindForeignMethodFn = bind;
  WrenVM* vm = wrenNewVM(&config);
  const char* source = probe ? "var result = 1+2\n" :
    "class Host {\nforeign static entered()\n}\nHost.entered()\nwhile (true) {}\n";
  if (wrenInterpret(vm, "cancel", source) != WREN_RESULT_SUCCESS) return 3;
  wrenEnsureSlots(vm, 1);
  wrenGetVariable(vm, "cancel", "result", 0);
  if (wrenGetSlotDouble(vm, 0) != 3) return 4;
  wrenFreeVM(vm);
  puts("fresh_vm_result=3");
  return 0;
}
