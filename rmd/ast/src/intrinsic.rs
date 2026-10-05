use crate::{AST, Expression, Literal, SettingMode, Statement};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrinsicMarkerError {
    Duplicate,
    Invalid,
}

pub fn intrinsic_marker(ast: &AST, body: &[Statement]) -> Result<Option<u16>, IntrinsicMarkerError> {
    let markers = body
        .iter()
        .filter_map(|statement| match statement {
            Statement::Setting { name, mode, value } if name.as_str() == "__demir_intrin" => Some((*mode, *value)),
            _ => None,
        })
        .collect::<Vec<_>>();

    if markers.len() > 1 {
        return Err(IntrinsicMarkerError::Duplicate);
    }

    let Some((SettingMode::Assign, id)) = markers.first() else {
        if !markers.is_empty() {
            return Err(IntrinsicMarkerError::Invalid);
        }

        return Ok(None);
    };

    let Some(expression) = ast.get_expr(*id) else {
        return Ok(None);
    };

    let Expression::Literal(Literal::Num(number)) = expression else {
        return Err(IntrinsicMarkerError::Invalid);
    };

    if !number.is_finite() || *number < 0.0 || number.fract() != 0.0 || *number > u16::MAX as f32 {
        return Err(IntrinsicMarkerError::Invalid);
    }

    Ok(Some(*number as u16))
}
