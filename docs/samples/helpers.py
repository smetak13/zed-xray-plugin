import math

LIMIT = 10


def double(x):
    return x * 2


def scaled(values):
    result = [double(v) for v in values if v < LIMIT]
    root = math.isqrt(sum(result))
    return root


print("side effect")
scaled([1, 7, 30])
