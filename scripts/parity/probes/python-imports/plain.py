import mod2
import nsp.inner
from nsp import inner as ii
from nsp.inner import h
import billing
import legacy_billing
from billing import charge, run_model
from billing import charge as pay


def caller():
    mod2.f()
    nsp.inner.h()
    ii.h()
    h()
    mod2.Thing().go()


def reexport_calls():
    billing.charge(1)
    billing.run_model(2)
    legacy_billing.charge(3)
    charge(4)
    run_model(5)
    pay(6)
    billing.refund(7)
