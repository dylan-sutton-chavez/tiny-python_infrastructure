---
title: "Functions"
description: "First-class functions, lambdas, closures, generators."
---

Functions are the central abstraction in Edge Python. They're values: pass them around, return them, store them, compose them.

## def

```python
def add(a, b):
    return a + b

print(add(3, 4))
```

```text Output
7
```

### Default arguments

```python
def greet(name, greeting="Hello"):
    return f"{greeting}, {name}!"

print(greet("world"))
print(greet("world", "Hi"))
```

```text Output
Hello, world!
Hi, world!
```

### Keyword arguments

```python
def f(x, y, z):
    return x * 100 + y * 10 + z

print(f(1, 2, 3))
print(f(x=1, z=3, y=2))
print(f(1, z=3, y=2))
```

```text Output
123
123
123
```

### Variadic: *args and **kwargs

```python
def total(*nums):
    return sum(nums)

print(total(1, 2, 3))
print(total(*[10, 20, 30]))
```

```text Output
6
60
```

### Argument unpacking at the call site

```python
def f(a, b, c):
    return a + b + c

print(f(*[1, 2, 3]))
print(f(*[1, 2], 3))
print(f(**{"a": 1, "b": 2, "c": 3}))
print(f(1, **{"b": 2, "c": 3}))
```

```text Output
6
6
6
6
```

## lambda

Anonymous function. The body is a single expression.

```python
double = lambda x: x * 2
print(double(21))

add = lambda a, b: a + b
print(add(3, 4))

# With defaults
greet = lambda name, msg="Hi": f"{msg}, {name}"
print(greet("world"))
```

```text Output
42
7
Hi, world
```

## First-class functions

Functions are values. Store them, pass them, return them.

```python
ops = [abs, len, str]
print([f(-3) for f in ops])
```

```text Output
[3, 2, '-3']
```

```python
# Functions as dict values — replaces switch/case
handlers = {
    "add":  lambda a, b: a + b,
    "mul":  lambda a, b: a * b,
    "max":  max,
}

print(handlers["add"](3, 4))
print(handlers["mul"](3, 4))
print(handlers["max"](3, 4))
```

```text Output
7
12
4
```

## Higher-order functions

Functions that take or return functions.

```python
def apply(f, x):
    return f(x)

print(apply(lambda n: n * n, 5))
print(apply(abs, -10))
```

```text Output
25
10
```

```python
# Returning a function
def make_adder(n):
    return lambda x: x + n

add5 = make_adder(5)
add10 = make_adder(10)

print(add5(3))
print(add10(3))
```

```text Output
8
13
```

## Closures

Functions capture their enclosing scope by reference.

```python
def counter():
    count = 0
    def step():
        nonlocal count
        count += 1
        return count
    return step

tick = counter()
print(tick())
print(tick())
print(tick())
```

```text Output
1
2
3
```

```python
# Closures over loop variables — captured by reference
def make_adders(n):
    return [lambda x, i=i: x + i for i in range(n)]

add0, add1, add2 = make_adders(3)
print(add0(10), add1(10), add2(10))
```

```text Output
10 11 12
```

## Currying

Partial application built from nested lambdas or closures.

```python
add = lambda x: lambda y: x + y

print(add(3)(4))

add3 = add(3)
print(add3(10))
print(add3(100))
```

```text Output
7
13
103
```

```python
# Curry helper
def curry(f):
    return lambda x: lambda y: f(x, y)

cmul = curry(lambda a, b: a * b)
double = cmul(2)
triple = cmul(3)

print(double(7), triple(7))
```

```text Output
14 21
```

## Function composition

```python
def compose(*fns):
    def piped(x):
        for f in fns:
            x = f(x)
        return x
    return piped

# Reads left-to-right: double, then square
pipeline = compose(lambda n: n * 2, lambda n: n * n)

print(pipeline(3))     # (3 * 2) ** 2
print([pipeline(x) for x in [1, 2, 3]])
```

```text Output
36
[4, 16, 36]
```

## Recursion

```python
def factorial(n):
    if n < 2:
        return 1
    return n * factorial(n - 1)

print(factorial(10))
```

```text Output
3628800
```

```python
# Mutual recursion
def is_even(n):
    return True if n == 0 else is_odd(n - 1)

def is_odd(n):
    return False if n == 0 else is_even(n - 1)

print(is_even(10), is_odd(10))
```

```text Output
True False
```

<Note>
Pure functions called repeatedly with the same arguments are automatically
memoized after two hits. The VM detects purity (no I/O, no mutation, no
raise, no yield) and caches results in a per-function template table. You
write naive recursion; the runtime handles caching for you.
</Note>

## Generators

Functions with `yield` produce a sequence lazily.

```python
def squares(n):
    for i in range(n):
        yield i * i

for x in squares(5):
    print(x)
```

```text Output
0
1
4
9
16
```

```python
# Materialize a generator
def naturals(limit):
    n = 1
    while n <= limit:
        yield n
        n += 1

print(list(naturals(5)))
```

```text Output
[1, 2, 3, 4, 5]
```

### yield from

Delegate to another generator.

```python
def nums():
    yield from range(3)
    yield from [10, 20]

print(list(nums()))
```

```text Output
[0, 1, 2, 10, 20]
```

## Generator expressions

Generators inline:

```python
print(sum(x * x for x in range(5)))
print(max(i for i in [3, 1, 4, 1, 5]))
```

```text Output
30
5
```

## Decorators

A decorator is a function that wraps another function. Edge Python supports them syntactically:

```python
def trace(f):
    def wrapped(*args):
        print(f"calling with {args}")
        return f(*args)
    return wrapped

@trace
def add(a, b):
    return a + b

print(add(3, 4))
```

```text Output
calling with [3, 4]
7
```

Stacked decorators apply bottom-up:

```python
def double_result(f):
    return lambda *a: f(*a) * 2

def add_one(f):
    return lambda *a: f(*a) + 1

@double_result
@add_one
def base(x):
    return x

# base(5) -> add_one -> 6 -> double_result -> 12
print(base(5))
```

```text Output
12
```