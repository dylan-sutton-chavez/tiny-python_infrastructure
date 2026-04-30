---
title: "Syntax"
description: "Operators, literals, and language surface."
---

## Comments

```python
# Single-line comment
x = 1  # Trailing comment

"""
Triple-quoted strings used as
module-level documentation are
parsed but discarded.
"""
```

## Identifiers and assignment

Identifiers follow Python rules: letters, digits, underscores, plus any non-ASCII letter.

```python
counter = 0
café = "open"
π = 3.14159

# Multiple targets
a = b = c = 0
print(a, b, c)
```

```text Output
0 0 0
```

### Tuple unpacking

```python
a, b = 1, 2
print(a, b)

# Star pattern
first, *middle, last = [1, 2, 3, 4, 5]
print(first, middle, last)
```

```text Output
1 2
1 [2, 3, 4] 5
```

### Walrus operator

Assignment as expression. Useful in conditions and comprehensions.

```python
data = [1, 2, 3]
if (n := len(data)) > 0:
    print(n)
```

```text Output
3
```

## Numbers

```python
# Integers — arbitrary precision
print(2 ** 100)
print(0xDEAD_BEEF)
print(0o777)
print(0b1010_1010)
print(1_000_000)
```

```text Output
1267650600228229401496703205376
3735928559
511
170
1000000
```

```python
# Floats — IEEE-754
print(3.14)
print(1e-5)
print(.5)

# Mixed arithmetic coerces to float
print(2 + 3.0)
```

```text Output
3.14
0.00001
0.5
5.0
```

## Strings

```python
print('single')
print("double")
print("""triple
quoted""")
print(r'raw\n')           # backslash not escaped
print('hello' ' world')   # implicit concatenation
```

```text Output
single
double
triple
quoted
raw\n
hello world
```

### Escape sequences

```python
print('\n line break')
print('\t tab')
print('\x41 hex')
print('\u00e9 unicode')
```

### f-strings

```python
name = "world"
n = 42
print(f"hello {name}")
print(f"answer is {n + 1}")
print(f"{n:04d}")     # format spec
print(f"{{literal braces}}")
```

```text Output
hello world
answer is 43
0042
{literal braces}
```

## Booleans and None

```python
print(True, False, None)
print(bool(0), bool(1), bool(""), bool("x"))
print(not True)
```

```text Output
True False None
False True False True
False
```

## Operators

### Arithmetic

```python
print(7 + 3, 7 - 3, 7 * 3, 7 / 3)
print(7 // 3, 7 % 3, 2 ** 10)
print(-5, +5)
```

```text Output
10 4 21 2.3333333333333335
2 1 1024
-5 5
```

### Comparison and chaining

```python
print(1 < 2 < 3)        # chained
print(0 < 5 < 10)
print(1 == 1 == 1)
```

```text Output
True
True
True
```

### Logical

Short-circuiting `and` / `or` return the operand, not a coerced bool.

```python
print(True and "second")
print(0 or "fallback")
print(None or 0 or [] or "default")
```

```text Output
second
fallback
default
```

### Bitwise

```python
print(5 & 3, 5 | 3, 5 ^ 3, ~5)
print(1 << 4, 32 >> 2)
```

```text Output
1 7 6 -6
16 8
```

### Membership and identity

```python
print(2 in [1, 2, 3])
print('a' in {'a': 1})
print(None is None)
print(1 is not 2)
```

```text Output
True
True
True
True
```

### Augmented assignment

`+=  -=  *=  /=  //=  %=  **=  &=  |=  ^=  <<=  >>=`

```python
x = 10
x += 5
x *= 2
print(x)
```

```text Output
30
```

### Conditional expression

```python
x = 5
print("big" if x > 3 else "small")
```

```text Output
big
```

## Containers

### Lists

```python
xs = [1, 2, 3]
print(xs[0], xs[-1])
print(xs[1:3])
print(xs + [4, 5])
print(xs * 2)
```

```text Output
1 3
[2, 3]
[1, 2, 3, 4, 5]
[1, 2, 3, 1, 2, 3]
```

### Tuples

```python
t = (1, 2, 3)
print(t[1])
print((1,))      # singleton needs the comma
print(())        # empty tuple
```

```text Output
2
(1,)
()
```

### Dicts (insertion-ordered)

```python
d = {"a": 1, "b": 2}
print(d["a"])
d["c"] = 3
print(d)
print(list(d.keys()))
```

```text Output
1
{'a': 1, 'b': 2, 'c': 3}
['a', 'b', 'c']
```

### Sets

```python
s = {1, 2, 2, 3}
print(s)
print(2 in s)
print(set())     # empty set literal needs the function
```

```text Output
{1, 2, 3}
True
set()
```

### Slicing

```python
a = [1, 2, 3, 4, 5]
print(a[1:4])      # [start:stop]
print(a[:2])
print(a[3:])
print(a[::2])      # every 2nd
print(a[::-1])     # reversed
```

```text Output
[2, 3, 4]
[1, 2]
[4, 5]
[1, 3, 5]
[5, 4, 3, 2, 1]
```

## Comprehensions

```python
print([x * x for x in range(5)])
print([x for x in range(10) if x % 2 == 0])
print([(i, j) for i in range(2) for j in range(2)])
print({x: x * x for x in range(4)})
print({x % 3 for x in range(10)})
```

```text Output
[0, 1, 4, 9, 16]
[0, 2, 4, 6, 8]
[(0, 0), (0, 1), (1, 0), (1, 1)]
{0: 0, 1: 1, 2: 4, 3: 9}
{0, 1, 2}
```

Generator expressions consumed by reducers:

```python
print(sum(x * x for x in range(5)))
print(max(x for x in [3, 1, 4, 1, 5]))
```

```text Output
30
5
```

## Type annotations

Annotations parse for compatibility but the VM ignores them. They have no runtime effect.

```python
counter: int = 0
name: str = "edge"

def add(a: int, b: int) -> int:
    return a + b

print(add(3, 4))
```

```text Output
7
```