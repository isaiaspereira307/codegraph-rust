package demo

open class Base {
    open fun describe(): String = "base"
}

object Registry {
    fun register(name: String): Int = name.length
}
