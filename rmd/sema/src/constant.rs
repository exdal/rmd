use core::types::{ListEntry, Value};

use ast::{AST, BinaryOp, Expression, ExpressionId, Literal, UnaryOp};

/// `1 + 2`
pub fn fold(ast: &AST, expr_id: ExpressionId) -> Value {
    let Some(expr) = ast.get_expr(expr_id) else {
        return Value::Unevaluated;
    };

    match expr {
        Expression::Literal(literal) => match literal {
            Literal::Null => Value::Null,
            Literal::Num(v) => Value::Num(*v),
            Literal::String(s) => Value::Text(core::types::decode_string(s)),
            Literal::Resource(s) => Value::Resource(s.clone()),
        },

        Expression::Path(path) => Value::Path(path.clone()),

        Expression::Grouped(inner) => fold(ast, *inner),

        Expression::Unary { op, operand } => match (op, fold(ast, *operand)) {
            (UnaryOp::Neg, Value::Num(v)) => Value::Num(-v),
            (UnaryOp::Not, value) => Value::Num(if value.is_truthy() { 0.0 } else { 1.0 }),
            (UnaryOp::BitNot, Value::Num(v)) => Value::Num(!(v as i32 as u32) as f32),
            _ => Value::Unevaluated,
        },

        Expression::Binary { op, lhs_expr, rhs_expr } => fold_binary(*op, fold(ast, *lhs_expr), fold(ast, *rhs_expr)),

        Expression::List(args) => Value::List(
            args.iter()
                .map(|arg| ListEntry {
                    key: match (arg.key, arg.value) {
                        (Some(key), _) => fold(ast, key),
                        (None, Some(value)) => fold(ast, value),
                        (None, None) => Value::Null,
                    },
                    value: arg.key.and_then(|_| arg.value.map(|value| fold(ast, value))),
                })
                .collect(),
        ),

        _ => Value::Unevaluated,
    }
}

fn fold_binary(op: BinaryOp, lhs: Value, rhs: Value) -> Value {
    // `"a" + "b"`
    if let (BinaryOp::Add, Value::Text(a), Value::Text(b)) = (op, &lhs, &rhs) {
        return Value::Text(format!("{a}{b}"));
    }

    let (Value::Num(a), Value::Num(b)) = (&lhs, &rhs) else {
        return Value::Unevaluated;
    };

    let (a, b) = (*a, *b);
    let value = match op {
        BinaryOp::Add => a + b,
        BinaryOp::Sub => a - b,
        BinaryOp::Mul => a * b,
        BinaryOp::Div if b != 0.0 => a / b,
        BinaryOp::Mod if (b as i32) != 0 => (a as i32).checked_rem(b as i32).unwrap_or(0) as f32,
        BinaryOp::FloatMod if b != 0.0 => a % b,
        BinaryOp::Pow => a.powf(b),
        BinaryOp::BitAnd => ((a as i32) & (b as i32)) as f32,
        BinaryOp::BitOr => ((a as i32) | (b as i32)) as f32,
        BinaryOp::BitXor => ((a as i32) ^ (b as i32)) as f32,
        BinaryOp::ShiftLeft => (a as i32).checked_shl(b as u32).unwrap_or(0) as f32,
        BinaryOp::ShiftRight => (a as i32).checked_shr(b as u32).unwrap_or(0) as f32,
        BinaryOp::CompEq => bool_to_num(a == b),
        BinaryOp::CompNotEq => bool_to_num(a != b),
        BinaryOp::CompLess => bool_to_num(a < b),
        BinaryOp::CompGreater => bool_to_num(a > b),
        BinaryOp::CompLessEq => bool_to_num(a <= b),
        BinaryOp::CompGreaterEq => bool_to_num(a >= b),
        BinaryOp::CompThreeWay => (a > b) as i32 as f32 - (a < b) as i32 as f32,
        BinaryOp::LogicalAnd => bool_to_num(a != 0.0 && b != 0.0),
        BinaryOp::LogicalOr => bool_to_num(a != 0.0 || b != 0.0),
        _ => return Value::Unevaluated,
    };

    Value::Num(value)
}

fn bool_to_num(v: bool) -> f32 { if v { 1.0 } else { 0.0 } }

#[cfg(test)]
mod tests {
    use core::types::{ListEntry, Value};

    use ast::{Argument, Expression, ExpressionId, Literal};

    use super::fold;

    #[test]
    fn folds_grouped_associative_and_omitted_list_entries() {
        let ast = ast::AST::new(
            Vec::new(),
            vec![
                Expression::Literal(Literal::Num(1.0)),
                Expression::Grouped(ExpressionId::new(0).unwrap()),
                Expression::Literal(Literal::String("key".to_string())),
                Expression::Literal(Literal::Num(2.0)),
                Expression::List(vec![
                    Argument {
                        key: None,
                        value: Some(ExpressionId::new(1).unwrap()),
                    },
                    Argument {
                        key: Some(ExpressionId::new(2).unwrap()),
                        value: Some(ExpressionId::new(3).unwrap()),
                    },
                    Argument { key: None, value: None },
                ]),
            ],
        );

        assert_eq!(
            fold(&ast, ExpressionId::new(4).unwrap()),
            Value::List(vec![
                ListEntry {
                    key: Value::Num(1.0),
                    value: None,
                },
                ListEntry {
                    key: Value::Text("key".to_string()),
                    value: Some(Value::Num(2.0)),
                },
                ListEntry {
                    key: Value::Null,
                    value: None,
                },
            ])
        );
    }

    /// `name = "Joe\'s bar\n"` reaches the object tree decoded, not as raw source.
    #[test]
    fn folds_a_string_literal_with_its_escapes_decoded() {
        let ast = ast::AST::new(
            Vec::new(),
            vec![Expression::Literal(Literal::String(String::from(r"Joe\'s bar\n")))],
        );

        assert_eq!(
            fold(&ast, ExpressionId::new(0).unwrap()),
            Value::Text(String::from("Joe's bar\n"))
        );
    }
}
