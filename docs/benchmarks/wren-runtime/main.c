#define _POSIX_C_SOURCE 200809L
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <string.h>
#include "wren.h"
#if WREN_VERSION_NUMBER != 4000
#error "This benchmark requires Wren 0.4.0"
#endif
static void error(WrenVM* vm, WrenErrorType type, const char* module, int line, const char* message) {
  (void)vm; (void)type;
  fprintf(stderr, "%s:%d: %s\n", module ? module : "runtime", line, message);
}
static double now(void) {
  struct timespec t;
  clock_gettime(CLOCK_MONOTONIC, &t);
  return t.tv_sec + t.tv_nsec * 1e-9;
}
static int compare(const void* a, const void* b) {
  double x = *(const double*)a, y = *(const double*)b;
  return (x > y) - (x < y);
}
int main(void) {
  const char* selected = getenv("RUSH_BENCH_CASE");
  const char* operation = getenv("RUSH_BENCH_OPERATION");
  const char* items = getenv("RUSH_BENCH_ITEMS");
  const char* iterations = getenv("RUSH_BENCH_ITERS");
  if ((operation && strcmp(operation, "prepared_run")) ||
      (items && strcmp(items, "1000")) || (iterations && strcmp(iterations, "100")) ||
      (selected && strcmp(selected, "closure") && strcmp(selected, "collections") && strcmp(selected, "collections_lazy"))) {
    fputs("Supported: prepared_run, N=1000, iterations=100, closure/collections/collections_lazy\n", stderr);
    return 2;
  }
  WrenConfiguration config;
  wrenInitConfiguration(&config);
  config.errorFn = error;
  WrenVM* vm = wrenNewVM(&config);
  const char* source =
    "class Bench {\n"
    "static closure() {\nvar scale = Fn.new {|factor| Fn.new {|x| x * factor }}\nreturn scale.call(2).call(21)\n}\n"
    "static eager() {\nvar values = (0...1000).toList\nvar mapped = values.map {|x| x*2 }.toList\nvar filtered = mapped.where {|x| x%3 == 0 }.toList\nreturn filtered.reduce(0) {|sum,x| sum+x }\n}\n"
    "static lazy() {\nreturn (0...1000).map {|x| x*2 }.where {|x| x%3 == 0 }.reduce(0) {|sum,x| sum+x }\n}\n}\n";
  if (wrenInterpret(vm, "benchmark", source) != WREN_RESULT_SUCCESS) return 1;
  wrenEnsureSlots(vm, 1);
  wrenGetVariable(vm, "benchmark", "Bench", 0);
  WrenHandle* receiver = wrenGetSlotHandle(vm, 0);
  const char* names[] = {"closure", "collections", "collections_lazy"};
  const char* methods[] = {"closure()", "eager()", "lazy()"};
  puts("Wren 0.4.0; N=1000; default GC; no execution budget; VM/compilation excluded");
  puts("workload\toperation\titerations/sample\tmin_us\tmedian_us\tmax_us");
  for (int c = 0; c < 3; c++) {
    if (selected && strcmp(selected, names[c])) continue;
    WrenHandle* call = wrenMakeCallHandle(vm, methods[c]);
    double samples[7];
    for (int sample = -1; sample < 7; sample++) {
      int iterations = sample < 0 ? 10 : 100;
      double start = now();
      for (int i = 0; i < iterations; i++) {
        wrenSetSlotHandle(vm, 0, receiver);
        if (wrenCall(vm, call) != WREN_RESULT_SUCCESS) return 1;
        if (wrenGetSlotType(vm, 0) != WREN_TYPE_NUM ||
            wrenGetSlotDouble(vm, 0) != (c == 0 ? 42 : 333666)) return 2;
      }
      if (sample >= 0) samples[sample] = (now()-start)*1e6/iterations;
    }
    qsort(samples, 7, sizeof(double), compare);
    printf("%s\tprepared_run\t100\t%.3f\t%.3f\t%.3f\n", names[c], samples[0], samples[3], samples[6]);
    wrenReleaseHandle(vm, call);
  }
  wrenReleaseHandle(vm, receiver);
  wrenFreeVM(vm);
}
