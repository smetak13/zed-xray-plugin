function pair(n) {
  let a = 0; let b = 0;
  a = n; b = n * 2;
  return a + b;
}

function jump(n) {
  switch (n) {
    case 0:
      let x = 1;
      return x;
    case 1:
      return n;
  }
}

const sq = (x) => x * x;

pair(1);
jump(1);
sq(3);
