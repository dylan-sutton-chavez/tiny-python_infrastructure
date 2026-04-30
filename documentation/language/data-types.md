---
title: "Data types"
description: "Numbers, strings, sequences, mappings, sets, and None."
---

## Type checks

```python
print(type(42))
print(type(3.14))
print(type("hi"))
print(type([1, 2]))
print(type((1, 2)))
print(type({1, 2}))
print(type({"a": 1}))
print(type(None))
print(type(True))
```

```text Output
<class 'int'>
<class 'float'>
<class 'str'>
<class 'list'>
<class 'tuple'>
<class 'set'>
<class 'dict'>
<class 'NoneType'>
<class 'bool'>
```

```python
print(isinstance(42, int))
print(isinstance(True, int))         # bools are ints
print(isinstance(42, (str, int)))    # tuple of types
```

```text Output
True
True
True
```

## Integer

48-bit inline by default; transparently promoted to BigInt above ±2⁴⁷. Arithmetic preserves precision indefinitely.

```python
# Inline range
print(2 ** 47)

# Promoted to BigInt
print(2 ** 100)
print(10 ** 30)

# BigInt arithmetic stays exact
print(10 ** 20 + 1)
print(2 ** 64 * 2 ** 64)
```

```text Output
140737488355328
1267650600228229401496703205376
1000000000000000000000000000000
100000000000000000001
340282366920938463463374607431768211456
```

```python
# Modular exponentiation
print(pow(7, 13, 19))
print(divmod(17, 5))
```

```text Output
7
(3, 2)
```

## Float

IEEE-754 double precision. Mixed arithmetic with int coerces to float.

```python
print(0.1 + 0.2 == 0.3)
print(-0.0 == 0.0)
print(1 / 3)
print(round(2.5))      # banker's rounding
print(round(0.5))
print(round(1.55, 1))
```

```text Output
False
True
0.3333333333333333
2
0
1.6
```

## String

Strings are immutable. Indexing returns a single-character string.

```python
s = "hello"
print(s[0], s[-1])
print(s[1:4])
print(len(s))
print(s + " world")
print(s * 2)
print("ll" in s)
```

```text Output
h o
ell
5
hello world
hellohello
True
```

Iteration yields characters:

```python
for ch in "abc":
    print(ch)
```

```text Output
a
b
c
```

## List

Mutable sequence.

```python
xs = [1, 2, 3]
xs[0] = 99
xs.append(4)
print(xs)
print(len(xs))

# Aliasing — both names see mutation
ys = xs
ys.append(5)
print(xs)
```

```text Output
[99, 2, 3, 4]
4
[99, 2, 3, 4, 5]
```

```python
# Equality is structural
print([1, 2, 3] == [1, 2, 3])
print([1, [2, 3]] == [1, [2, 3]])
```

```text Output
True
True
```

## Tuple

Immutable sequence. The fastest container for fixed-size data and the only one usable as a dict key in mixed-type cases.

```python
t = (1, 2, 3)
print(t[0])
print(t + (4, 5))
print((1,))         # one-element needs trailing comma
print(())           # empty
```

```text Output
1
(1, 2, 3, 4, 5)
(1,)
()
```

## Dict

Insertion-ordered mapping. Keys must be hashable (numbers, strings, tuples of hashables).

```python
d = {"a": 1, "b": 2}
print(d["a"])
d["c"] = 3
print(d)
print(list(d.keys()))
print(list(d.values()))
print(list(d.items()))
```

```text Output
1
{'a': 1, 'b': 2, 'c': 3}
['a', 'b', 'c']
[1, 2, 3]
[('a', 1), ('b', 2), ('c', 3)]
```

```python
# Iteration yields keys
for k in {"x": 1, "y": 2}:
    print(k)
```

```text Output
x
y
```

## Set

Unordered collection of hashable values, no duplicates.

```python
s = {1, 2, 2, 3}
print(s)
print(len(s))

# Empty set literal is set(), not {}
print(set())
print(type({}))     # this is a dict
```

```text Output
{1, 2, 3}
3
set()
<class 'dict'>
```

## Range

Lazy integer sequence. `range(stop)`, `range(start, stop)`, `range(start, stop, step)`.

```python
print(list(range(5)))
print(list(range(2, 8)))
print(list(range(0, 10, 2)))
print(list(range(10, 0, -1)))
```

```text Output
[0, 1, 2, 3, 4]
[2, 3, 4, 5, 6, 7]
[0, 2, 4, 6, 8]
[10, 9, 8, 7, 6, 5, 4, 3, 2, 1]
```

## NoneType

Single value, single type.

```python
print(None)
print(None is None)
print(type(None))
```

```text Output
None
True
<class 'NoneType'>
```

## Conversions

```python
print(int("42"))
print(int(3.7))         # truncates toward zero
print(int(True))
print(float("3.14"))
print(str(42))
print(str([1, 2]))
print(bool([]))         # empty is falsy
print(bool([0]))        # non-empty is truthy
print(list("abc"))
print(tuple([1, 2, 3]))
print(set([1, 1, 2]))
```

```text Output
42
3
1
3.14
42
[1, 2]
False
True
['a', 'b', 'c']
(1, 2, 3)
{1, 2}
```

## Truthy and falsy

These values are falsy. Everything else is truthy.

| Falsy values        |
|---------------------|
| `None`              |
| `False`             |
| `0`, `0.0`          |
| `""` (empty string) |
| `[]`, `()`          |
| `{}`, `set()`       |
| `range(0)`          |