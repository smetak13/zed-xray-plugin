function countPairs(n) {
  let total = 0;
  for (let i = 0; i < n; i++) {
    let j = 0;
    while (j < i) {
      total += j;
      j += 1;
    }
  }
  return total;
}

countPairs(3);
