#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "wren.h"
#if WREN_VERSION_NUMBER != 4000
#error "Requires Wren 0.4.0"
#endif

/* The host owns both the lifetime control block and its optional payload.
 * The foreign object stores a borrowed control pointer, never a payload pointer.
 * This single-object fixture checks foreign identity before reading its memory. */
typedef struct { double* owner; void* point; const char* expected; int matched; } State;
typedef struct { State* state; } Point;

static void fail(WrenVM* vm, const char* message) {
  wrenSetSlotString(vm, 0, message);
  wrenAbortFiber(vm, 0);
}
static State* point_state(WrenVM* vm, int slot) {
  State* state = wrenGetUserData(vm);
  if (wrenGetSlotType(vm, slot) != WREN_TYPE_FOREIGN ||
      wrenGetSlotForeign(vm, slot) != state->point) {
    fail(vm, "expected Point");
    return NULL;
  }
  Point* point = wrenGetSlotForeign(vm, slot);
  if (!point->state->owner) {
    fail(vm, "point was deleted");
    return NULL;
  }
  return point->state;
}
static void allocate_point(WrenVM* vm) {
  State* state = wrenGetUserData(vm);
  Point* point = wrenSetSlotNewForeign(vm, 0, 0, sizeof(Point));
  point->state = state;
  state->point = point;
}
static void allocate_other(WrenVM* vm) {
  wrenSetSlotNewForeign(vm, 0, 0, 1);
}
static void read_point(WrenVM* vm) {
  State* state = point_state(vm, 1);
  if (state) wrenSetSlotDouble(vm, 0, *state->owner);
}
static void get(WrenVM* vm) {
  State* state = point_state(vm, 0);
  if (state) wrenSetSlotDouble(vm, 0, *state->owner);
}
static void move_point(WrenVM* vm) {
  State* state = point_state(vm, 0);
  if (!state) return;
  if (wrenGetSlotType(vm, 1) != WREN_TYPE_NUM) {
    fail(vm, "expected number");
    return;
  }
  double next = *state->owner + wrenGetSlotDouble(vm, 1);
  if (!isfinite(next)) {
    fail(vm, "position must be finite");
    return;
  }
  *state->owner = next;
  wrenSetSlotNull(vm, 0);
}
static WrenForeignClassMethods bind_class(WrenVM* vm, const char* module, const char* name) {
  (void)vm;
  WrenForeignClassMethods methods = {0};
  if (!strcmp(module, "objects")) {
    if (!strcmp(name, "Point")) methods.allocate = allocate_point;
    if (!strcmp(name, "Other")) methods.allocate = allocate_other;
  }
  return methods;
}
static WrenForeignMethodFn bind_method(WrenVM* vm, const char* module,
    const char* name, bool is_static, const char* signature) {
  (void)vm;
  if (strcmp(module, "objects")) return NULL;
  if (is_static && !strcmp(name, "Host") && !strcmp(signature, "read(_)")) return read_point;
  if (!is_static && !strcmp(name, "Point")) {
    if (!strcmp(signature, "get()")) return get;
    if (!strcmp(signature, "move(_)")) return move_point;
  }
  return NULL;
}
static void error(WrenVM* vm, WrenErrorType type, const char* module, int line, const char* message) {
  (void)module; (void)line;
  State* state = wrenGetUserData(vm);
  if (type == WREN_ERROR_RUNTIME && state->expected && !strcmp(message, state->expected))
    state->matched = 1;
  else if (type != WREN_ERROR_STACK_TRACE) fprintf(stderr, "Unexpected error: %s\n", message);
}
static int rejects(WrenVM* vm, const char* source, const char* message) {
  State* state = wrenGetUserData(vm);
  state->expected = message;
  state->matched = 0;
  WrenInterpretResult result = wrenInterpret(vm, "objects", source);
  state->expected = NULL;
  return result == WREN_RESULT_RUNTIME_ERROR && state->matched;
}
int main(void) {
  State state = {0};
  state.owner = malloc(sizeof(double));
  if (!state.owner) return 1;
  *state.owner = 2;
  WrenConfiguration config;
  wrenInitConfiguration(&config);
  config.userData = &state;
  config.bindForeignClassFn = bind_class;
  config.bindForeignMethodFn = bind_method;
  config.errorFn = error;
  WrenVM* vm = wrenNewVM(&config);
  int status = 1;
  const char* source =
    "foreign class Point {\nconstruct new() {}\nforeign get()\nforeign move(delta)\n}\n"
    "foreign class Other {\nconstruct new() {}\n}\n"
    "class Host {\nforeign static read(point)\n}\n"
    "var point = Point.new()\nvar alias = point\npoint.move(3)\nvar result = Host.read(alias)\n";
  if (wrenInterpret(vm, "objects", source) != WREN_RESULT_SUCCESS) goto cleanup;
  wrenEnsureSlots(vm, 1);
  wrenGetVariable(vm, "objects", "result", 0);
  if (wrenGetSlotType(vm, 0) != WREN_TYPE_NUM || wrenGetSlotDouble(vm, 0) != 5 || *state.owner != 5) goto cleanup;
  if (!rejects(vm, "Host.read(42)\n", "expected Point") ||
      !rejects(vm, "Host.read(Other.new())\n", "expected Point") ||
      !rejects(vm, "point.move(\"bad\")\n", "expected number") ||
      !rejects(vm, "point.move(1/0)\n", "position must be finite") || *state.owner != 5) goto cleanup;
  free(state.owner);
  state.owner = NULL;
  if (!rejects(vm, "point.get()\n", "point was deleted") ||
      !rejects(vm, "alias.get()\n", "point was deleted") ||
      !rejects(vm, "Host.read(alias)\n", "point was deleted") ||
      !rejects(vm, "alias.move(1)\n", "point was deleted")) goto cleanup;
  if (wrenInterpret(vm, "objects", "result = 1+2\n") != WREN_RESULT_SUCCESS) goto cleanup;
  wrenEnsureSlots(vm, 1);
  wrenGetVariable(vm, "objects", "result", 0);
  if (wrenGetSlotType(vm, 0) != WREN_TYPE_NUM || wrenGetSlotDouble(vm, 0) != 3) goto cleanup;
  puts("Wren 0.4.0: alias mutation, wrong argument and foreign types, nonfinite rejection, deleted owner and VM reuse passed");
  status = 0;
cleanup:
  wrenFreeVM(vm);
  free(state.owner);
  return status;
}
