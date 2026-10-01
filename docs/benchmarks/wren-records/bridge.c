#include <math.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
#include "wren.h"
#include "records_script.h"
#if WREN_VERSION_NUMBER != 4000
#error "Requires Wren 0.4.0"
#endif
typedef int (*Next)(void*, const char**, size_t*, double*, bool*);
typedef int (*Emit)(void*, const char*, size_t, double);
typedef struct { void* context; Next next; Emit emit; } State;
static void next_record(WrenVM* vm) {
  State* state = wrenGetUserData(vm);
  const char* group = NULL;
  size_t length = 0;
  double amount = 0;
  bool active = false;
  int status = state->next(state->context, &group, &length, &amount, &active);
  if (status < 0) {
    wrenSetSlotString(vm, 0, "input error");
    wrenAbortFiber(vm, 0);
  } else if (!status) {
    wrenSetSlotNull(vm, 0);
  } else {
    wrenEnsureSlots(vm, 4);
    wrenSetSlotNewList(vm, 0);
    wrenSetSlotBytes(vm, 1, group, length);
    wrenSetSlotDouble(vm, 2, amount);
    wrenSetSlotBool(vm, 3, active);
    for (int slot = 1; slot <= 3; slot++) wrenInsertInList(vm, 0, -1, slot);
  }
}
static void finite_number(WrenVM* vm) {
  wrenSetSlotBool(vm, 0, wrenGetSlotType(vm, 1) == WREN_TYPE_NUM && isfinite(wrenGetSlotDouble(vm, 1)));
}
static WrenForeignMethodFn bind(WrenVM* vm, const char* module, const char* name, bool is_static, const char* signature) {
  (void)vm;
  if (strcmp(module, "records") || strcmp(name, "Host") || !is_static) return NULL;
  if (!strcmp(signature, "next()")) return next_record;
  if (!strcmp(signature, "finite(_)")) return finite_number;
  return NULL;
}
static void error(WrenVM* vm, WrenErrorType type, const char* module, int line, const char* message) {
  (void)vm; (void)type;
  fprintf(stderr, "%s:%d: %s\n", module ? module : "records", line, message);
}
int rush_wren_records(void* context, Next next, Emit emit) {
  State state = {context, next, emit};
  WrenConfiguration config;
  wrenInitConfiguration(&config);
  config.userData = &state;
  config.bindForeignMethodFn = bind;
  config.errorFn = error;
  WrenVM* vm = wrenNewVM(&config);
  int status = 1;
  if (wrenInterpret(vm, "records", script) != WREN_RESULT_SUCCESS) goto done;
  wrenEnsureSlots(vm, 5);
  wrenGetVariable(vm, "records", "result", 0);
  if (wrenGetSlotType(vm, 0) != WREN_TYPE_LIST) goto done;
  int count = wrenGetListCount(vm, 0);
  for (int i = 0; i < count; i++) {
    wrenGetListElement(vm, 0, i, 1);
    if (wrenGetSlotType(vm, 1) != WREN_TYPE_MAP) goto done;
    wrenSetSlotString(vm, 2, "group");
    wrenGetMapValue(vm, 1, 2, 3);
    wrenSetSlotString(vm, 2, "total");
    wrenGetMapValue(vm, 1, 2, 4);
    if (wrenGetSlotType(vm, 3) != WREN_TYPE_STRING || wrenGetSlotType(vm, 4) != WREN_TYPE_NUM) goto done;
    int length = 0;
    const char* group = wrenGetSlotBytes(vm, 3, &length);
    double total = wrenGetSlotDouble(vm, 4);
    if (length < 0 || !isfinite(total) || !emit(context, group, (size_t)length, total)) goto done;
  }
  status = 0;
done:
  wrenFreeVM(vm);
  return status;
}
