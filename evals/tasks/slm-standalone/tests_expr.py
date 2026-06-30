import pytest
from expr import eval_expr


def test_basic_arithmetic():
    assert eval_expr("1 + 2") == 3
    assert eval_expr("5 - 3") == 2
    assert eval_expr("2 * 4") == 8
    assert eval_expr("9 / 3") == 3.0


def test_precedence():
    assert eval_expr("1 + 2 * 3") == 7
    assert eval_expr("(1 + 2) * 3") == 9
    assert eval_expr("1 - 2 + 3") == 2
    assert eval_expr("2 / 2 + 4") == 5.0


def test_unary_minus():
    assert eval_expr("-3") == -3
    assert eval_expr("1 - -3") == 4
    assert eval_expr("-(2 + 1)") == -3


def test_nested_parentheses():
    assert eval_expr("((1))") == 1
    assert eval_expr("(1 + (2 + 3))") == 6
    assert eval_expr("(((9)) / 3)") == 3.0


def test_float_numbers():
    assert eval_expr("3.5 + 2") == 5.5
    assert eval_expr("2 * 1.5") == 3.0
    assert eval_expr("4.25 / 0.5") == 8.5


def test_error_cases():
    with pytest.raises(ValueError):
        eval_expr("")
    with pytest.raises(ValueError):
        eval_expr(")(")
    with pytest.raises(ValueError):
        eval_expr("1 + * 2")
    with pytest.raises(ValueError):
        eval_expr("1 / 0")

if __name__ == "__main__":
    pytest.main([__file__])