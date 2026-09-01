import { parseRef, parseRange } from './refs.js';

const ERRORS = ['#DIV/0!', '#VALUE!', '#REF!', '#NAME?', '#N/A', '#NUM!', '#CIRC!'];

/* ------------------------------------------------------------------ lexer */

export function tokenize(input) {
  const src = String(input ?? '');
  const tokens = [];
  let i = 0;

  const peekErr = () => ERRORS.find((e) => src.startsWith(e, i));

  while (i < src.length) {
    const c = src[i];

    if (c === ' ' || c === '\t' || c === '\n' || c === '\r') {
      i++;
      continue;
    }

    const err = peekErr();
    if (err) {
      tokens.push({ type: 'error', value: err });
      i += err.length;
      continue;
    }

    if (c === '"') {
      let j = i + 1;
      let str = '';
      while (j < src.length) {
        if (src[j] === '"') {
          if (src[j + 1] === '"') {
            str += '"';
            j += 2;
            continue;
          }
          break;
        }
        str += src[j++];
      }
      if (j >= src.length) throw new FormulaError('#VALUE!', 'unterminated string');
      tokens.push({ type: 'string', value: str });
      i = j + 1;
      continue;
    }

    if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(src[i + 1] ?? ''))) {
      const m = src.slice(i).match(/^\d*\.?\d+(?:[eE][+-]?\d+)?/);
      tokens.push({ type: 'number', value: Number(m[0]) });
      i += m[0].length;
      continue;
    }

    // A run of letters/digits/$ may be a range, a ref, a boolean, or a name.
    if (/[A-Za-z_$À-￿]/.test(c)) {
      const m = src.slice(i).match(/^[A-Za-z_$À-￿][A-Za-z0-9_.$À-￿]*/);
      let word = m[0];
      let rest = i + word.length;

      // Range: WORD ':' WORD
      const rangeMatch = src.slice(i).match(/^\$?[A-Za-z]{1,3}\$?\d{1,7}[ \t]*:[ \t]*\$?[A-Za-z]{1,3}\$?\d{1,7}/);
      if (rangeMatch && parseRange(rangeMatch[0].replace(/\s/g, ''))) {
        tokens.push({ type: 'range', value: rangeMatch[0].replace(/\s/g, '').toUpperCase() });
        i += rangeMatch[0].length;
        continue;
      }

      const upper = word.toUpperCase();
      if (upper === 'TRUE' || upper === 'FALSE') {
        tokens.push({ type: 'boolean', value: upper === 'TRUE' });
        i = rest;
        continue;
      }

      if (parseRef(word)) {
        tokens.push({ type: 'ref', value: word.toUpperCase() });
        i = rest;
        continue;
      }

      tokens.push({ type: 'name', value: word });
      i = rest;
      continue;
    }

    const two = src.slice(i, i + 2);
    if (two === '<=' || two === '>=' || two === '<>') {
      tokens.push({ type: 'op', value: two });
      i += 2;
      continue;
    }

    if ('+-*/^&=<>%'.includes(c)) {
      tokens.push({ type: 'op', value: c });
      i++;
      continue;
    }
    if (c === '(') {
      tokens.push({ type: 'lparen' });
      i++;
      continue;
    }
    if (c === ')') {
      tokens.push({ type: 'rparen' });
      i++;
      continue;
    }
    if (c === ',' || c === ';') {
      tokens.push({ type: 'comma' });
      i++;
      continue;
    }
    if (c === '{' || c === '}') {
      tokens.push({ type: c === '{' ? 'lbrace' : 'rbrace' });
      i++;
      continue;
    }

    throw new FormulaError('#VALUE!', `unexpected character "${c}"`);
  }
  return tokens;
}

export class FormulaError extends Error {
  constructor(code, message) {
    super(message ?? code);
    this.code = code;
    this.name = 'FormulaError';
  }
}

/* ----------------------------------------------------------------- parser */

const BINARY_PRECEDENCE = {
  '=': 1, '<>': 1, '<': 1, '>': 1, '<=': 1, '>=': 1,
  '&': 2,
  '+': 3, '-': 3,
  '*': 4, '/': 4,
  '^': 5,
};

/**
 * Pratt parser producing a small AST.
 * Node kinds: number|string|boolean|error|ref|range|name|call|binary|unary|percent
 */
export function parse(formula) {
  const text = String(formula ?? '').replace(/^\s*=/, '');
  const tokens = tokenize(text);
  let pos = 0;

  const peek = () => tokens[pos];
  const next = () => tokens[pos++];
  const expect = (type) => {
    const t = next();
    if (!t || t.type !== type) throw new FormulaError('#VALUE!', `expected ${type}`);
    return t;
  };

  function parseExpr(minPrec = 0) {
    let left = parseUnary();
    for (;;) {
      const t = peek();
      if (!t || t.type !== 'op') break;
      const prec = BINARY_PRECEDENCE[t.value];
      if (prec === undefined || prec < minPrec) break;
      next();
      // '^' is right-associative in spreadsheets.
      const right = parseExpr(t.value === '^' ? prec : prec + 1);
      left = { kind: 'binary', op: t.value, left, right };
    }
    return left;
  }

  function parseUnary() {
    const t = peek();
    if (t?.type === 'op' && (t.value === '-' || t.value === '+')) {
      next();
      return { kind: 'unary', op: t.value, arg: parseUnary() };
    }
    return parsePostfix();
  }

  function parsePostfix() {
    let node = parsePrimary();
    while (peek()?.type === 'op' && peek().value === '%') {
      next();
      node = { kind: 'percent', arg: node };
    }
    return node;
  }

  function parsePrimary() {
    const t = next();
    if (!t) throw new FormulaError('#VALUE!', 'unexpected end of formula');

    switch (t.type) {
      case 'number':
        return { kind: 'number', value: t.value };
      case 'string':
        return { kind: 'string', value: t.value };
      case 'boolean':
        return { kind: 'boolean', value: t.value };
      case 'error':
        return { kind: 'error', value: t.value };
      case 'ref':
        return { kind: 'ref', ref: t.value };
      case 'range':
        return { kind: 'range', range: t.value };
      case 'lparen': {
        const e = parseExpr(0);
        expect('rparen');
        return e;
      }
      case 'lbrace': {
        // Inline array {1,2;3} — flattened, enough for SUM({1,2,3}).
        const items = [];
        while (peek() && peek().type !== 'rbrace') {
          items.push(parseExpr(0));
          if (peek()?.type === 'comma') next();
        }
        expect('rbrace');
        return { kind: 'array', items };
      }
      case 'name': {
        if (peek()?.type === 'lparen') {
          next();
          const args = [];
          if (peek()?.type !== 'rparen') {
            for (;;) {
              args.push(peek()?.type === 'comma' ? { kind: 'blank' } : parseExpr(0));
              if (peek()?.type === 'comma') {
                next();
                continue;
              }
              break;
            }
          }
          expect('rparen');
          return { kind: 'call', name: t.value.toUpperCase(), args };
        }
        return { kind: 'name', name: t.value };
      }
      default:
        throw new FormulaError('#VALUE!', `unexpected token ${t.type}`);
    }
  }

  const ast = parseExpr(0);
  if (pos < tokens.length) throw new FormulaError('#VALUE!', 'trailing input');
  return ast;
}

/** Every cell address a formula reads, ranges expanded lazily by the caller. */
export function collectRefs(ast, out = { refs: [], ranges: [], names: [] }) {
  if (!ast || typeof ast !== 'object') return out;
  switch (ast.kind) {
    case 'ref':
      out.refs.push(ast.ref);
      break;
    case 'range':
      out.ranges.push(ast.range);
      break;
    case 'name':
      out.names.push(ast.name);
      break;
    case 'binary':
      collectRefs(ast.left, out);
      collectRefs(ast.right, out);
      break;
    case 'unary':
    case 'percent':
      collectRefs(ast.arg, out);
      break;
    case 'call':
      ast.args.forEach((a) => collectRefs(a, out));
      break;
    case 'array':
      ast.items.forEach((a) => collectRefs(a, out));
      break;
    default:
      break;
  }
  return out;
}

export const ERROR_CODES = ERRORS;
