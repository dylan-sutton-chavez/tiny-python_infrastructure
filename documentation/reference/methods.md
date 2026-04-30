---
title: "Methods"
description: "Built-in methods on strings, lists, and dicts."
---

Edge Python has no user-defined classes, but strings, lists, and dicts come with a curated set of built-in methods. They behave like CPython equivalents.

```python
# Methods are accessed with dot notation
print("hello".upper())
print([3, 1, 2].count(1))
print({"a": 1}.get("a"))
```

```text Output
HELLO
1
1
```

## String methods

### Case transforms

```python
print("hello".upper())
print("HELLO".lower())
print("hello world".capitalize())
print("hello world".title())
```

```text Output
HELLO
hello
Hello world
Hello World
```

### Whitespace

```python
print("  hi  ".strip())
print("  hi  ".lstrip())
print("  hi  ".rstrip())

# With a custom strip set
print("xxhelloxx".strip("x"))
```

```text Output
hi
hi  
  hi
hello
```

### Predicates

```python
print("123".isdigit())
print("abc".isdigit())

print("abc".isalpha())
print("abc123".isalpha())

print("abc123".isalnum())
print("abc 123".isalnum())
```

```text Output
True
False
True
False
True
False
```

### Search and count

```python
print("hello".startswith("he"))
print("hello".endswith("lo"))
print("hello".find("ll"))
print("hello".find("z"))
print("hello".count("l"))
```

```text Output
True
True
2
-1
2
```

### Split, join, replace

```python
print("a,b,c".split(","))
print("hello world".split()) # any whitespace
print(",".join(["a", "b", "c"]))
print("hello".replace("l", "L"))
```

```text Output
['a', 'b', 'c']
['hello', 'world']
a,b,c
heLLo
```

### Padding

```python
print("abc".center(7, "-"))
print("42".zfill(5))
print("-42".zfill(5))
```

```text Output
--abc--
00042
-0042
```

## List methods

### Pure (return a new value or query)

```python
xs = [1, 2, 3, 2]

print(xs.index(2))
print(xs.count(2))

ys = xs.copy()
ys.append(99)
print(xs) # original unchanged
print(ys)
```

```text Output
1
2
[1, 2, 3, 2]
[1, 2, 3, 2, 99]
```

### Mutating

These return `None` and modify the list in place.

```python
xs = [1, 2, 3]

xs.append(4)
print(xs)

xs.extend([5, 6])
print(xs)

xs.insert(0, 99)
print(xs)
```

```text Output
[1, 2, 3, 4]
[1, 2, 3, 4, 5, 6]
[99, 1, 2, 3, 4, 5, 6]
```

```python
xs = [1, 2, 3, 2]

xs.remove(2) # first occurrence
print(xs)

popped = xs.pop() # last
print(popped, xs)

popped = xs.pop(0) # by index
print(popped, xs)
```

```text Output
[1, 3, 2]
2 [1, 3]
1 [3]
```

```python
xs = [3, 1, 4, 1, 5]
xs.sort()
print(xs)

xs.reverse()
print(xs)

xs.clear()
print(xs)
```

```text Output
[1, 1, 3, 4, 5]
[5, 4, 3, 1, 1]
[]
```

## Dict methods

### Views

```python
d = {"a": 1, "b": 2, "c": 3}

print(list(d.keys()))
print(list(d.values()))
print(list(d.items()))
```

```text Output
['a', 'b', 'c']
[1, 2, 3]
[('a', 1), ('b', 2), ('c', 3)]
```

### Lookup with default

```python
d = {"a": 1}

print(d.get("a"))
print(d.get("z"))
print(d.get("z", 0))
```

```text Output
1
None
0
```

### Mutation

```python
d = {"a": 1}

d.update({"b": 2, "a": 99})
print(d)

removed = d.pop("a")
print(removed, d)

print(d.pop("missing", "fallback"))
```

```text Output
{'a': 99, 'b': 2}
99 {'b': 2}
fallback
```

```python
d = {}
d.setdefault("a", 1)
d.setdefault("a", 999) # second call ignored
print(d)
```

```text Output
{'a': 1}
```

## Method summary

### String — `str`

| Method        | Arity   | Returns                              |
|---------------|---------|--------------------------------------|
| `upper`       | 0       | uppercased copy                      |
| `lower`       | 0       | lowercased copy                      |
| `capitalize`  | 0       | first letter upper, rest lower       |
| `title`       | 0       | each word capitalized                |
| `strip`       | 0       | no leading/trailing whitespace       |
| `lstrip`      | 0 or 1  | left-strip; optional set of chars    |
| `rstrip`      | 0 or 1  | right-strip; optional set of chars   |
| `isdigit`     | 0       | bool: all ASCII digits               |
| `isalpha`     | 0       | bool: all alphabetic                 |
| `isalnum`     | 0       | bool: all alphanumeric               |
| `startswith`  | 1       | bool                                 |
| `endswith`    | 1       | bool                                 |
| `find`        | 1       | index or -1                          |
| `count`       | 1       | non-overlapping occurrences          |
| `split`       | 0 or 1  | list of pieces                       |
| `join`        | 1       | joined string                        |
| `replace`     | 2       | new string with all replacements     |
| `center`      | 1 or 2  | padded copy                          |
| `zfill`       | 1       | zero-padded copy, sign-aware         |

### List — `list`

| Method        | Arity   | Mutates? | Returns                       |
|---------------|---------|----------|-------------------------------|
| `index`       | 1       | no       | first matching index          |
| `count`       | 1       | no       | matching count                |
| `copy`        | 0       | no       | shallow copy                  |
| `append`      | 1       | yes      | None                          |
| `extend`      | 1       | yes      | None                          |
| `insert`      | 2       | yes      | None                          |
| `remove`      | 1       | yes      | None                          |
| `pop`         | 0 or 1  | yes      | popped value                  |
| `sort`        | 0       | yes      | None                          |
| `reverse`     | 0       | yes      | None                          |
| `clear`       | 0       | yes      | None                          |

### Dict — `dict`

| Method        | Arity   | Mutates? | Returns                       |
|---------------|---------|----------|-------------------------------|
| `keys`        | 0       | no       | list of keys                  |
| `values`      | 0       | no       | list of values                |
| `items`       | 0       | no       | list of `(k, v)` tuples       |
| `get`         | 1 or 2  | no       | value or default              |
| `update`      | 1       | yes      | None                          |
| `pop`         | 1 or 2  | yes      | popped value or default       |
| `setdefault`  | 1 or 2  | yes      | existing or default value     |