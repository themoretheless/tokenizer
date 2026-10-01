#include <stdio.h>
#include "wren.h"
#if WREN_VERSION_NUMBER != 4000
#error "Requires Wren 0.4.0"
#endif
int main(void) {
  WrenConfiguration config;
  wrenInitConfiguration(&config);
  WrenVM* vm = wrenNewVM(&config);
  int status = 1;
  if (wrenInterpret(vm, "startup", "var result = 1+2\n") == WREN_RESULT_SUCCESS) {
    wrenEnsureSlots(vm, 1);
    wrenGetVariable(vm, "startup", "result", 0);
    if (wrenGetSlotType(vm, 0) == WREN_TYPE_NUM && wrenGetSlotDouble(vm, 0) == 3) {
      puts("3");
      status = 0;
    }
  }
  wrenFreeVM(vm);
  return status;
}
