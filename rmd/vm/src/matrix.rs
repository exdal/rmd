use core::{path::TreePath, types::Identifier};

use codegen::opcode::Binary;
use objtree::ObjectTree;

use crate::{
    FaultKind,
    GenericValue,
    eval::Evaluator,
    heap::{Heap, Object, ObjectId},
};

type Result<T> = std::result::Result<T, crate::Fault>;

const MATRIX_COPY: u32 = 0;
const MATRIX_MULTIPLY: u32 = 1;
const MATRIX_ADD: u32 = 2;
const MATRIX_SUBTRACT: u32 = 3;
const MATRIX_INVERT: u32 = 4;
const MATRIX_ROTATE: u32 = 5;
const MATRIX_SCALE: u32 = 6;
const MATRIX_TRANSLATE: u32 = 7;
const MATRIX_INTERPOLATE: u32 = 8;
const MATRIX_MODIFY: u32 = 128;

const FIELDS: [&str; 6] = ["a", "b", "c", "d", "e", "f"];

/// BYOND's `/matrix`: `x' = a*x + b*y + c` and `y' = d*x + e*y + f`, with y pointing north.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix(pub [f32; 6]);

impl Default for Matrix {
    fn default() -> Self { Self::IDENTITY }
}

impl Matrix {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);

    pub fn scale(x: f32, y: f32) -> Self { Self([x, 0.0, 0.0, 0.0, y, 0.0]) }

    pub fn translate(x: f32, y: f32) -> Self { Self([1.0, 0.0, x, 0.0, 1.0, y]) }

    /// `Turn()` is clockwise
    pub fn turn(degrees: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();

        Self([cos, sin, 0.0, -sin, cos, 0.0])
    }

    /// `A * B` applies `A` first
    pub fn then(self, next: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [na, nb, nc, nd, ne, nf] = next.0;

        Self([
            na * a + nb * d,
            na * b + nb * e,
            na * c + nb * f + nc,
            nd * a + ne * d,
            nd * b + ne * e,
            nd * c + ne * f + nf,
        ])
    }

    pub fn invert(self) -> Option<Self> {
        let [a, b, c, d, e, f] = self.0;
        let determinant = a * e - b * d;
        if determinant == 0.0 || !determinant.is_finite() {
            return None;
        }

        Some(Self([
            e / determinant,
            -b / determinant,
            (b * f - c * e) / determinant,
            -d / determinant,
            a / determinant,
            (c * d - a * f) / determinant,
        ]))
    }

    fn zip(self, other: Self, op: impl Fn(f32, f32) -> f32) -> Self {
        Self(std::array::from_fn(|index| op(self.0[index], other.0[index])))
    }

    /// BYOND blends the scale, rotation and translation separately, not the six components.
    pub fn interpolate(self, other: Self, t: f32) -> Self {
        let (from, to) = (self.decompose(), other.decompose());
        let lerp = |from: f32, to: f32| from + (to - from) * t;
        let mut turn = to.turn - from.turn;
        if turn > 180.0 {
            turn -= 360.0;
        } else if turn < -180.0 {
            turn += 360.0;
        }

        Self::scale(lerp(from.scale[0], to.scale[0]), lerp(from.scale[1], to.scale[1]))
            .then(Self::turn(from.turn + turn * t))
            .then(Self::translate(
                lerp(from.translate[0], to.translate[0]),
                lerp(from.translate[1], to.translate[1]),
            ))
    }

    fn decompose(self) -> Decomposed {
        let [a, b, c, d, e, f] = self.0;
        let scale_x = a.hypot(b);
        let scale_y = if scale_x == 0.0 {
            d.hypot(e)
        } else {
            (a * e - b * d) / scale_x
        };

        Decomposed {
            scale: [scale_x, scale_y],
            turn: b.atan2(a).to_degrees(),
            translate: [c, f],
        }
    }

    pub fn is_identity(self) -> bool { self == Self::IDENTITY }
}

struct Decomposed {
    scale: [f32; 2],
    turn: f32,
    translate: [f32; 2],
}

pub(crate) fn read(heap: &Heap, tree: &ObjectTree, value: &GenericValue) -> Option<Matrix> {
    let object = value.object().and_then(|id| heap.object(id))?;
    let root = tree.id_of(&TreePath::parse("/matrix"))?;
    if !tree.is_subtype_of(object.ty, root) {
        return None;
    }

    let mut matrix = Matrix::IDENTITY;
    for (slot, name) in matrix.0.iter_mut().zip(FIELDS) {
        let name = Identifier::from(name);
        let value = match object.vars.get(&name) {
            Some(value) => value.num(),
            None => tree
                .var_inherited(object.ty, &name)
                .and_then(|variable| variable.value.as_num()),
        };
        if let Some(value) = value {
            *slot = value;
        }
    }

    Some(matrix)
}

impl Evaluator<'_> {
    pub(crate) fn matrix(&self, value: &GenericValue) -> Option<Matrix> { read(&self.runtime.heap, self.tree, value) }

    pub(crate) fn store_matrix(&mut self, target: Option<ObjectId>, matrix: Matrix) -> Result<GenericValue> {
        let id = match target {
            Some(id) => id,
            None => {
                let ty = self
                    .tree
                    .id_of(&TreePath::parse("/matrix"))
                    .ok_or_else(|| self.fault(FaultKind::MissingVariable("/matrix".into())))?;
                self.reserve(1)?;

                self.runtime
                    .heap
                    .alloc_object(Object::new(ty))
                    .map_err(|kind| self.fault(kind))?
            },
        };

        for (value, name) in matrix.0.into_iter().zip(FIELDS) {
            self.write_field(GenericValue::Object(id), name.into(), GenericValue::Num(value))?;
        }

        Ok(GenericValue::Object(id))
    }

    pub(crate) fn matrix_call(&mut self, args: Vec<GenericValue>) -> Result<GenericValue> {
        let Some(first) = args.first() else {
            return self.store_matrix(None, Matrix::IDENTITY);
        };

        let Some(source) = self.matrix(first) else {
            if args.len() == 6 {
                let mut matrix = Matrix::IDENTITY;
                for (slot, value) in matrix.0.iter_mut().zip(&args) {
                    *slot = self.number(value)?;
                }
                return self.store_matrix(None, matrix);
            }

            let flags = self.number(&args[args.len() - 1])? as u32;
            let x = self.number(first)?;
            let y = match args.len() {
                3 => self.number(&args[1])?,
                _ => x,
            };
            let matrix = match flags & !MATRIX_MODIFY {
                MATRIX_ROTATE => Matrix::turn(x),
                MATRIX_SCALE => Matrix::scale(x, y),
                MATRIX_TRANSLATE => Matrix::translate(x, y),
                _ => return Err(self.fault(FaultKind::InvalidOperation("matrix() form".into()))),
            };
            return self.store_matrix(None, matrix);
        };

        if args.len() == 1 {
            return self.store_matrix(None, source);
        }

        let flags = self.number(&args[args.len() - 1])? as u32;
        let operands = &args[1..args.len() - 1];
        let operand = |index: usize| operands.get(index).cloned().unwrap_or_default();
        let result = match flags & !MATRIX_MODIFY {
            MATRIX_COPY => self.matrix(&operand(0)).unwrap_or(source),
            MATRIX_MULTIPLY => match self.matrix(&operand(0)) {
                Some(other) => source.then(other),
                None => {
                    let factor = self.number(&operand(0))?;
                    source.then(Matrix::scale(factor, factor))
                },
            },
            op @ (MATRIX_ADD | MATRIX_SUBTRACT) => {
                let other = self
                    .matrix(&operand(0))
                    .ok_or_else(|| self.fault(FaultKind::InvalidOperation("expected matrix".into())))?;
                match op {
                    MATRIX_ADD => source.zip(other, |left, right| left + right),
                    _ => source.zip(other, |left, right| left - right),
                }
            },
            MATRIX_INVERT => source.invert().unwrap_or(source),
            MATRIX_ROTATE => source.then(Matrix::turn(self.number(&operand(0))?)),
            op @ (MATRIX_SCALE | MATRIX_TRANSLATE) => {
                let x = self.number(&operand(0))?;
                let y = match operands.get(1) {
                    Some(value) => self.number(value)?,
                    None => x,
                };
                match op {
                    MATRIX_SCALE => source.then(Matrix::scale(x, y)),
                    _ => source.then(Matrix::translate(x, y)),
                }
            },
            MATRIX_INTERPOLATE => {
                let other = self
                    .matrix(&operand(0))
                    .ok_or_else(|| self.fault(FaultKind::InvalidOperation("expected matrix".into())))?;
                source.interpolate(other, self.number(&operand(1))?)
            },
            _ => return Err(self.fault(FaultKind::InvalidOperation("matrix() form".into()))),
        };

        let target = (flags & MATRIX_MODIFY != 0).then(|| first.object()).flatten();
        self.store_matrix(target, result)
    }

    pub(crate) fn matrix_binary(
        &mut self, op: Binary, left: &GenericValue, right: &GenericValue,
    ) -> Result<Option<GenericValue>> {
        let (matrix, other) = match (self.matrix(left), self.matrix(right)) {
            (Some(left), Some(right)) => (left, Ok(right)),
            (Some(left), None) => (left, Err(right)),
            // `2 * M`
            (None, Some(right)) if op == Binary::Mul => (right, Err(left)),
            _ => return Ok(None),
        };

        let invalid = |this: &Self| this.fault(FaultKind::InvalidOperation("matrix operator".into()));
        let result = match (op, other) {
            (Binary::Mul, Ok(other)) => matrix.then(other),
            (Binary::Div, Ok(other)) => matrix.then(other.invert().ok_or_else(|| invalid(self))?),
            (Binary::Add, Ok(other)) => matrix.zip(other, |left, right| left + right),
            (Binary::Sub, Ok(other)) => matrix.zip(other, |left, right| left - right),
            (Binary::Mul, Err(factor)) => {
                let factor = self.number(factor)?;
                matrix.then(Matrix::scale(factor, factor))
            },
            (Binary::Div, Err(divisor)) => {
                let divisor = self.number(divisor)?;
                if divisor == 0.0 {
                    return Err(invalid(self));
                }
                matrix.then(Matrix::scale(1.0 / divisor, 1.0 / divisor))
            },
            _ => return Err(invalid(self)),
        };

        self.store_matrix(None, result).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::Matrix;

    fn close(left: Matrix, right: Matrix) -> bool {
        left.0
            .iter()
            .zip(right.0)
            .all(|(left, right)| (left - right).abs() < 1e-4)
    }

    #[test]
    fn turning_east_by_ninety_degrees_points_south() {
        let [a, b, c, d, e, f] = Matrix::turn(90.0).0;
        let east = [1.0, 0.0];

        let turned = [a * east[0] + b * east[1] + c, d * east[0] + e * east[1] + f];
        assert!((turned[0]).abs() < 1e-6 && (turned[1] + 1.0).abs() < 1e-6);
    }

    #[test]
    fn then_applies_the_left_matrix_first() {
        let scaled_then_moved = Matrix::scale(2.0, 2.0).then(Matrix::translate(3.0, 0.0));

        assert_eq!(scaled_then_moved, Matrix([2.0, 0.0, 3.0, 0.0, 2.0, 0.0]));
    }

    #[test]
    fn an_inverted_matrix_undoes_the_original() {
        let matrix = Matrix::turn(30.0)
            .then(Matrix::scale(2.0, 3.0))
            .then(Matrix::translate(-4.0, 5.0));

        assert!(close(
            matrix.then(matrix.invert().expect("invertible")),
            Matrix::IDENTITY
        ));
        assert_eq!(Matrix::scale(0.0, 1.0).invert(), None);
    }

    #[test]
    fn interpolation_turns_rather_than_shrinking_through_the_middle() {
        let halfway = Matrix::IDENTITY.interpolate(Matrix::turn(90.0), 0.5);

        assert!(close(halfway, Matrix::turn(45.0)));
    }
}
