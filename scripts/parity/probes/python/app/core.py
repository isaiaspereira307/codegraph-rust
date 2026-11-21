import os
import json as js
from typing import List, Optional
from .util import helper, other as oth
from . import util
from ..pkg import thing
from app.models import Model
import app.models


CONST = helper(1)
setup()


class Base:
    def run(self):
        return self.step()

    def step(self):
        return 1


class Child(Base, Mixin, metaclass=Meta):
    x: int = make_default()

    def __init__(self, repo: "Repo"):
        super().__init__()
        Base.__init__(self)
        self.repo = repo
        self.items: List[int] = []

    def run(self):
        self.repo.save(self)
        self.step()
        cls_method()
        data = js.loads("{}")
        os.path.join("a", "b")
        return [x.upper() for x in self.items]

    @staticmethod
    def build() -> "Child":
        return Child(None)

    @property
    def size(self) -> int:
        return len(self.items)

    class Inner(Base):
        def go(self):
            return helper(2)


def uses_callbacks(items):
    sorted(items, key=sort_key)
    items.sort(key=lambda v: v)
    threading.Thread(target=worker).start()
    register(worker, on_done=finish)
    callbacks = [worker, finish]
    handler = worker
    return map(sort_key, items)


def sort_key(v):
    return v


def worker():
    pass


def finish():
    pass


def outer():
    def inner():
        return worker()
    lam = lambda: finish()
    return inner()


async def aio():
    await worker()
    async with ctx() as c:
        pass
    result = await c.fetch()
    return result


def guarded():
    if False:
        worker()
    if TYPE_CHECKING:
        finish()


def annotated(a: Base, b: Optional[Child] = None) -> Base:
    return a


@decorator
@other.deco(arg=1)
def decorated():
    pass


def chained():
    return Builder().with_x(1).build().run()


def calls_on_call():
    get_handler()()
    obj.attr.method()
    obj["k"].method()
    return getattr(obj, "x")()


if __name__ == "__main__":
    main()
