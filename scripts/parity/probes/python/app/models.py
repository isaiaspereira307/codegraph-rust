from dataclasses import dataclass


@dataclass
class Model:
    name: str

    def save(self):
        return persist(self)


def persist(m):
    return m
