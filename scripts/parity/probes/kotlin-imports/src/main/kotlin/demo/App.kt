package demo

import demo.Helper

class App : Base() {
    fun run(helper: Helper): Int = helper.twice(2)

    fun build(): Int {
        val fresh: Helper = Helper()
        Registry.register("app")
        return fresh.twice(3)
    }
}
