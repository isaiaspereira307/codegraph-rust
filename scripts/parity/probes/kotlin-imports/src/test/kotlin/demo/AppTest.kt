package demo

import demo.App
import demo.Helper
import org.junit.Test

class AppTest {
    @Test
    fun runsTwice() {
        App().run(Helper())
    }

    fun testRun() {
        val app = App()
        app.build()
    }
}
