---
layout: doc
outline: deep
lang: en-US
---

# COMBINA function

## Overview
The **COMBINA** function returns the number of combinations for a set of items when repetitions are allowed. It is useful when you need to determine how many unique groups of a specific size can be formed from a larger pool of items where each item can be selected more than once.

## Usage
### Syntax
**COMBINA(<span title="Number" style="color:#1E88E5">number</span>, <span title="Number" style="color:#1E88E5">number_chosen</span>) => <span title="Number" style="color:#1E88E5">combinations</span>**

### Argument descriptions
* *number* ([number](/features/value-types#numbers), required). The total number of distinct items available. Values with decimals are truncated to integers.
* *number_chosen* ([number](/features/value-types#numbers), required). The number of items in each combination. Values with decimals are truncated to integers.

### Additional guidance
If you need combinations without repetition, use the [COMBIN](https://support.microsoft.com/office/combin-function-89073f40-2f70-4fb9-8b82-4901034f0b34) function instead. When order matters, use [PERMUT](/functions/statistical/permut) or a related permutation function.

### Returned value
COMBINA returns a [number](/features/value-types#numbers) representing the count of possible combinations with repetition.

### Error conditions
* If either argument is omitted or more than two arguments are supplied, COMBINA returns the [`#ERROR!`](/features/error-types.md#error) error.
* If either argument cannot be interpreted as a [number](/features/value-types#numbers), COMBINA returns the [`#VALUE!`](/features/error-types.md#value) error.
* If either argument is negative, or the result exceeds the numeric range handled by IronCalc, COMBINA returns the [`#NUM!`](/features/error-types.md#num) error.

## Details
COMBINA calculates the value of the binomial coefficient $\binom{n + k - 1}{k}$, where $n$ is *number* and $k$ is *number_chosen*. When *number* is zero and *number_chosen* is greater than zero, COMBINA returns zero, reflecting that no combinations are possible. When both arguments are zero, COMBINA returns one to represent the empty combination.

## Examples
* `COMBINA(10, 3)` returns `220`, counting all 3-item codes that can be formed with digits 0–9 when digits can repeat.
* `COMBINA(4, 2)` returns `10`, showing the number of 2-item selections that can be made from four items when repetition is allowed.
* `COMBINA(0, 3)` returns `0`, because there are no items to choose from.

## Links
* [Microsoft Excel documentation for COMBINA](https://support.microsoft.com/office/combina-function-205c1f8f-2323-4d1c-ac54-53398b3e0ad3)
* [Wikipedia: Multiset coefficient](https://en.wikipedia.org/wiki/Multiset#Counting_multisets)
