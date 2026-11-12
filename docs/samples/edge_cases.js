function sequence(n) {
  let a = 1;
  return a++, a + n;
}

function blockFunction(n) {
  let a = n;
  if (n) {
    function g() { return 1; }
    a = g();
  }
  a += 1;
  return a;
}

function shadowing(x) {
  let y = x + 1;
  {
    y = y * 2;
    let x = y;
    y = x + 1;
  }
  return y;
}

sequence(1);
blockFunction(1);
shadowing(1);
