from typing import TYPE_CHECKING
from pkg import b
from pkg.b import Thing, helper as hh
import pkg.b as bm


def free():
    pass


class K:
    @classmethod
    def make(cls):
        return cls.build()

    def build(self):
        def nested():
            return self.make()
        return nested()

    def m(self):
        x.free()
        bm.helper()
        hh()
        Thing.go()
        Thing().go()
        b.Thing.go()
        K.make()
        obj.cb = free
        d = {"k": free}
        t = (free, 1)
        if TYPE_CHECKING:
            free()
        if not True:
            free()
        if 0:
            pass
        else:
            free()
        while False:
            free()
        return free
