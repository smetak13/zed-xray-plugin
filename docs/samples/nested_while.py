def count_pairs(n):
    total = 0
    for i in range(n):
        j = 0
        while j < i:
            total += j
            j += 1
    return total


count_pairs(3)
