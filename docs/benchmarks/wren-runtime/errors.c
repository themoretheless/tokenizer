#include <stdio.h>
#include <string.h>
#include "wren.h"
#if WREN_VERSION_NUMBER != 4000
#error "Requires Wren 0.4.0"
#endif
static int compile_errors, runtime_errors, source_frames, host_message;
static const char* expected_module = "syntax-case";
static int expected_line = 2;
static void error(WrenVM* vm, WrenErrorType type, const char* module, int line, const char* message) {
  (void)vm;
  if (type == WREN_ERROR_COMPILE) compile_errors++;
  if (type == WREN_ERROR_RUNTIME) runtime_errors++;
  if (module && !strcmp(module, expected_module) && line == expected_line) source_frames++;
  if (!strcmp(message, "host rejected operation")) host_message = 1;
  printf("type=%d module=%s line=%d %s\n", type, module ? module : "<none>", line, message);
}
static void fail(WrenVM* vm) {
  wrenSetSlotString(vm, 0, "host rejected operation");
  wrenAbortFiber(vm, 0);
}
static WrenForeignMethodFn bind(WrenVM* vm, const char* module, const char* class_name,
                                bool is_static, const char* signature) {
  (void)vm; (void)module;
  return is_static && !strcmp(class_name,"Host") && !strcmp(signature,"fail()") ? fail : NULL;
}
static int reuse(WrenVM* vm, const char* module) {
  if (wrenInterpret(vm,module,"var result = 1+2\n") != WREN_RESULT_SUCCESS) return 0;
  wrenEnsureSlots(vm,1);
  wrenGetVariable(vm,module,"result",0);
  return wrenGetSlotType(vm,0)==WREN_TYPE_NUM && wrenGetSlotDouble(vm,0)==3;
}
int main(void) {
  WrenConfiguration config;
  wrenInitConfiguration(&config);
  config.errorFn=error;
  config.bindForeignMethodFn=bind;
  WrenVM* vm=wrenNewVM(&config);
  if (wrenInterpret(vm,"syntax-case","var value =\n") != WREN_RESULT_COMPILE_ERROR ||
      !compile_errors || !source_frames || !reuse(vm,"after-syntax")) return 1;
  source_frames=0;
  expected_module="runtime-case";
  if (wrenInterpret(vm,"runtime-case","var value = null\nvalue.missing\n") != WREN_RESULT_RUNTIME_ERROR ||
      !runtime_errors || !source_frames || !reuse(vm,"after-runtime")) return 2;
  runtime_errors=source_frames=0;
  expected_module="host-case"; expected_line=4;
  if (wrenInterpret(vm,"host-case","class Host {\nforeign static fail()\n}\nHost.fail()\n") != WREN_RESULT_RUNTIME_ERROR ||
      !runtime_errors || !source_frames || !host_message || !reuse(vm,"after-host")) return 3;
  wrenFreeVM(vm);
  puts("Wren 0.4.0: compile/runtime/foreign errors, source callbacks and VM reuse passed");
}
