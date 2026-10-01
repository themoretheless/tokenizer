class Host {
  foreign static next()
  foreign static finite(value)
}
var totals = {}
var order = []
while (true) {
  var row = Host.next()
  if (row == null) break
  if (row[2]) {
    var key = row[0]
    if (!totals.containsKey(key)) {
      totals[key] = 0
      order.add(key)
    }
    var next = totals[key] + row[1]
    if (!Host.finite(next)) Fiber.abort("total must be finite")
    totals[key] = next
  }
}
var result = []
for (key in order) result.add({"group":key, "total":totals[key]})
