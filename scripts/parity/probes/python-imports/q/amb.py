from q.lib import g
from q import lib
import q.lib
import q


def g():
    pass


def run(x):
    return x


class A:
    class Inner:
        def m(self):
            return self.n()

        def n(self):
            def deep():
                return self.m()
            return deep()


class B:
    class Inner:
        def m(self):
            return self.m()


def use():
    g()
    lib.f()
    lib()
    run(lib)
    q.lib.f()
    q.lib.Svc().go()
    lib.Svc.go()
