def eval_expr(s: str) -> float:
    if not s:
        raise ValueError("Empty expression")

    # Remove whitespace (but preserve correct handling of unary minus)
    tokens = []
    num_start = None

    i = 0
    while i < len(s):
        c = s[i]

        if c.isspace():
            i += 1
            continue

        if c in "+-*/()":
            # Handle unary minus: if this is a '-' and we're at the start or after an operator,
            # it's likely a unary minus
            tokens.append(c)
            i += 1
        elif c.isdigit() or c == '.':
            # Number (int or float)
            if num_start is None:
                num_start = i
            i += 1
            if i == len(s) or not (s[i].isdigit() or s[i] == '.'):
                # End of number
                tokens.append(s[num_start:i])
                num_start = None
        else:
            raise ValueError(f"Invalid character '\u0027{c}\u0027' at position {i}")
    
    if num_start is not None:
        tokens.append(s[num_start:])

    # Now parse the tokens using recursive descent

    def parse_expr(pos=0):
        """Parse an expression (addition and subtraction)"""
        
        # Parse the first term
        num, pos = parse_term(pos)

        while pos < len(tokens) and tokens[pos] in "+-":
            op = tokens[pos]
            pos += 1
            next_num, pos = parse_term(pos)

            if op == "+":
                num += next_num
            elif op == "-":
                num -= next_num
        
        return num, pos
    
    def parse_term(pos=0):
        """Parse a term (multiplication and division)"""
        
        # Parse the first factor
        num, pos = parse_factor(pos)

        while pos < len(tokens) and tokens[pos] in "*/":
            op = tokens[pos]
            pos += 1
            next_num, pos = parse_factor(pos)

            if op == "*":
                num *= next_num
            elif op == "/":
                if next_num == 0:
                    raise ValueError("Division by zero")
                num /= next_num
        
        return num, pos
    
    def parse_factor(pos=0):
        """Parse a factor (numbers or parenthesized expressions)"""
        
        token = tokens[pos]

        if isinstance(token, str) and token[0].isdigit():
            # It's a number, just convert and return
            return float(token), pos + 1

        elif token == "(":
            # It's an inner expression
            num, pos = parse_expr(pos + 1)
            if pos >= len(tokens) or tokens[pos] != ")":
                raise ValueError("Mismatched parentheses")
            return num, pos + 1
        
        elif token == "-":
            # Handle unary minus - recursively get the number and negate it
            next_num, pos = parse_factor(pos + 1)
            return -next_num, pos
        
        else:
            raise ValueError(f"Unexpected token '\u0027{token}\u0027' at position {pos}")
    
    # Start parsing from the beginning of tokens
    result, pos = parse_expr(0)

    # Should have reached end of input by now
    if pos != len(tokens):
        raise ValueError("Unexpected trailing characters")
    
    return float(result)