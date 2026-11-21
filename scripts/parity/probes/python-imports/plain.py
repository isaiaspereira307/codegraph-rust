import mod2
import nsp.inner
from nsp import inner as ii
from nsp.inner import h


def caller():
    mod2.f()
    nsp.inner.h()
    ii.h()
    h()
    mod2.Thing().go()
