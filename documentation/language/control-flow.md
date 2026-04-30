---
title: "Control flow"
description: "Conditionals, loops, exceptions, pattern matching."
---

## if / elif / else

```python
def classify(n):
    if n < 0:
        return "negative"
    elif n == 0:
        return "zero"
    else:
        return "positive"

for x in [-3, 0, 7]:
    print(classify(x))
```

```text Output
negative
zero
positive
```

## while

```python
n, total = 5, 0
while n > 0:
    total += n
    n -= 1
print(total)
```

```text Output
15
```

### while ... else

The `else` runs if the loop completes without `break`.

```python
x = 0
while x < 3:
    x += 1
else:
    print("loop finished cleanly")
```

```text Output
loop finished cleanly
```

## for

Iterates anything that produces a sequence: list, tuple, dict, set, range, string, generator.

```python
for ch in "abc":
    print(ch)
```

```text Output
a
b
c
```

```python
# Tuple unpacking in the loop variable
pairs = [("a", 1), ("b", 2), ("c", 3)]
for key, value in pairs:
    print(key, value)
```

```text Output
a 1
b 2
c 3
```

```python
# Star pattern works too
for first, *rest in [[1, 2, 3], [4, 5, 6, 7]]:
    print(first, rest)
```

```text Output
1 [2, 3]
4 [5, 6, 7]
```

### break and continue

```python
for i in range(10):
    if i == 5:
        break
    if i % 2 == 0:
        continue
    print(i)
```

```text Output
1
3
```

### for ... else

Runs when the loop exhausts its iterator (no `break`).

```python
for i in range(3):
    pass
else:
    print("done")
```

```text Output
done
```

## match / case

Pattern matching by equality and the `_` wildcard.

```python
def describe(n):
    match n:
        case 0:
            return "zero"
        case 1:
            return "one"
        case _:
            return "many"

for x in [0, 1, 2, 99]:
    print(describe(x))
```

```text Output
zero
one
many
many
```

## try / except / else / finally

```python
def safe_div(a, b):
    try:
        return a / b
    except ZeroDivisionError:
        return None

print(safe_div(10, 2))
print(safe_div(10, 0))
```

```text Output
5.0
None
```

```python
# Multiple handlers and finally
try:
    x = int("abc")
except ValueError:
    x = -1
finally:
    print("cleanup")
print(x)
```

```text Output
cleanup
-1
```

```python
# Bare except catches everything
try:
    raise "boom"
except:
    print("caught")
```

```text Output
caught
```

### raise

```python
def positive(n):
    if n < 0:
        raise ValueError
    return n

try:
    positive(-1)
except ValueError:
    print("rejected")
```

```text Output
rejected
```

`raise X from Y` chains exceptions:

```python
try:
    raise ValueError from KeyError
except:
    print("caught chain")
```

```text Output
caught chain
```

### Exception names available

These are pre-bound type names you can match against:

`Exception`, `BaseException`, `ValueError`, `TypeError`, `NameError`, `KeyError`, `IndexError`, `AttributeError`, `RuntimeError`, `ZeroDivisionError`, `OverflowError`, `MemoryError`, `RecursionError`, `StopIteration`, `NotImplementedError`, `OSError`, `IOError`, `ImportError`, `ModuleNotFoundError`, `AssertionError`, `ArithmeticError`, `LookupError`.

## with

Context managers. `__enter__` / `__exit__` aren't user-definable in Edge Python, but the syntax is supported for compatibility:

```python
x = [1, 2]
with x as items:
    print(len(items))
print("after")
```

```text Output
2
after
```

Multiple targets:

```python
a, b = "first", "second"
with a as x, b as y:
    print(x, y)
```

```text Output
first second
```

## assert

```python
def reciprocal(n):
    assert n != 0
    return 1 / n

print(reciprocal(4))
```

```text Output
0.25
```

A failed assertion raises `AssertionError`.

## del

Removes a binding from the slot. Works on plain names and indexed positions.

```python
x = 42
del x
try:
    print(x)
except NameError:
    print("gone")
```

```text Output
gone
```