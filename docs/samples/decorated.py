import functools


@functools.cache
def square(n):
    result = n * n
    return result


square(4)
