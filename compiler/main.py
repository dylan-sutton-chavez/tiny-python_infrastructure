def fac(n):
    r = 1
    for i in range(2, n + 1):
        r *= i
    return r

def isqrt(n):
    if n == 0: return 0
    x = n
    for _ in range(300):
        y = (x + n // x) // 2
        if y >= x:
            return x
        x = y
    return x

def pi(digits):
    terms = digits // 14 + 3
    C3 = 640320 ** 3

    S_num = 0
    S_den = 1

    for k in range(terms):
        a = fac(6*k) * (13591409 + 545140134 * k)
        b = fac(3*k) * fac(k)**3 * C3**k
        if k % 2 == 0:
            S_num = S_num * b + a * S_den
        else:
            S_num = S_num * b - a * S_den
        S_den = S_den * b

    scale = 10 ** (digits + 20)
    sq = isqrt(10005 * scale * scale)
    return 426880 * sq * S_den // (S_num * scale) // 10**20

print(pi(1000))