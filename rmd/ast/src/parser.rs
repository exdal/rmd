use core::{
    location::Location,
    path::{PathFlags, TreePath},
    types::{Identifier, VarModifiers},
};

use lexer::token::{SoftKeyword, Token};

use crate::{
    AST,
    AccessKind,
    Argument,
    AssignmentKind,
    BinaryOp,
    Builtin,
    Declaration,
    Expression,
    ExpressionId,
    ForLoop,
    InputType,
    Literal,
    LoopBinding,
    PickValue,
    ProcKind,
    ProcParam,
    SettingMode,
    Statement,
    SwitchCase,
    SwitchValue,
    TypeSpec,
    UnaryOp,
    VarSpec,
    error::ParseError,
    precedence::{Precedence, token_to_assignment_kind, token_to_binary_op, token_to_precedence, token_to_unary_op},
};

pub type ParseResult<T> = Result<T, ParseError>;

struct PathModifier {
    flags: PathFlags,
    /// `var`/`proc`/`verb`
    declares: bool,
    /// `/datum/task/throw`
    may_be_name: bool,
}

fn path_modifier(keyword: SoftKeyword) -> Option<PathModifier> {
    let (flags, declares, may_be_name) = match keyword {
        SoftKeyword::Proc => (PathFlags::IS_PROC, true, true),
        SoftKeyword::Verb => (PathFlags::IS_VERB, true, true),
        SoftKeyword::Var => (PathFlags::IS_VAR, true, false),
        SoftKeyword::Global => (PathFlags::IS_GLOBAL, false, false),
        SoftKeyword::Static => (PathFlags::IS_STATIC, false, true),
        SoftKeyword::Const => (PathFlags::IS_CONST, false, true),
        SoftKeyword::Tmp => (PathFlags::IS_TMP, false, true),
        SoftKeyword::Final => (PathFlags::IS_FINAL, false, true),
        _ => return None,
    };

    Some(PathModifier {
        flags,
        declares,
        may_be_name,
    })
}

fn soft_builtin(keyword: SoftKeyword) -> Option<Builtin> {
    Some(match keyword {
        SoftKeyword::Args => Builtin::Args,
        SoftKeyword::Callee => Builtin::Callee,
        SoftKeyword::Caller => Builtin::Caller,
        SoftKeyword::Global => Builtin::Global,
        SoftKeyword::Src => Builtin::Src,
        SoftKeyword::Usr => Builtin::Usr,
        SoftKeyword::World => Builtin::World,
        _ => return None,
    })
}

pub struct Parser<'a, 't> {
    tokens: &'t [(Token<'a>, Location)],
    cursor: usize,
    expressions: Vec<Expression>,
    /// `a ? b:c : d`
    ternary_true_depth: usize,
}

impl<'a, 't> Parser<'a, 't> {
    pub fn new(tokens: &'t [(Token<'a>, Location)]) -> Self {
        Self {
            tokens,
            cursor: 0,
            expressions: Vec::new(),
            ternary_true_depth: 0,
        }
    }

    pub fn make_expr(&mut self, expr: Expression) -> ExpressionId {
        let expr_id = ExpressionId::new(self.expressions.len());
        self.expressions.push(expr);

        expr_id
    }

    pub fn parse(&mut self) -> ParseResult<AST> {
        let mut declarations = Vec::new();

        self.skip_declaration_delimiters();
        while self.peek().is_some() && !self.peek_is(Token::Eof) {
            self.parse_declaration(None, &mut declarations)?;
            self.skip_declaration_delimiters();
        }

        Ok(AST::new(declarations, std::mem::take(&mut self.expressions)))
    }

    fn peek(&self) -> Option<(Token<'a>, Location)> { self.tokens.get(self.cursor).copied() }

    fn peek_at(&self, look_ahead: usize) -> Option<(Token<'a>, Location)> {
        self.tokens.get(self.cursor + look_ahead).copied()
    }

    fn peek_is(&self, token: Token<'_>) -> bool { self.peek().is_some_and(|(t, _)| t == token) }

    fn peek_at_is(&self, look_ahead: usize, token: Token<'_>) -> bool {
        self.peek_at(look_ahead).is_some_and(|(t, _)| t == token)
    }

    fn token_touches_next(&self, offset: usize) -> bool {
        let Some((_, left)) = self.peek_at(offset) else {
            return false;
        };

        let Some((token, right)) = self.peek_at(offset + 1) else {
            return false;
        };

        token.word().is_some() && left.file == right.file && left.end == right.begin
    }

    fn advance(&mut self) -> ParseResult<(Token<'a>, Location)> {
        let entry = self.peek().ok_or_else(ParseError::end_of_file)?;
        self.cursor += 1;

        Ok(entry)
    }

    fn expect_next(&mut self, expected: Token<'_>) -> ParseResult<(Token<'a>, Location)> {
        let (token, location) = self.advance()?;
        if token != expected {
            return Err(ParseError::unexpected(expected, token, location));
        }

        Ok((token, location))
    }

    fn consume(&mut self, token: Token<'_>) -> bool {
        if self.peek_is(token) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn consume_step(&mut self) -> bool { self.consume(Token::Soft(SoftKeyword::Step)) }

    fn skip_newlines(&mut self) {
        while self.peek_is(Token::Newline) {
            self.cursor += 1;
        }
    }

    fn skip_statement_delimiters(&mut self) {
        while self.peek_is(Token::Newline) || self.peek_is(Token::Semicolon) {
            self.cursor += 1;
        }
    }

    fn skip_declaration_delimiters(&mut self) {
        while self.peek_is(Token::Newline)
            || self.peek_is(Token::Semicolon)
            || self.peek().is_some_and(|(token, _)| token.is_comment())
        {
            self.cursor += 1;
        }
    }

    fn at_statement_end(&self) -> bool {
        self.peek().is_none_or(|(token, _)| {
            matches!(
                token,
                Token::Newline | Token::Semicolon | Token::Dedent | Token::BraceRight | Token::Eof
            )
        })
    }

    fn parse_block(&mut self, prefix: Option<&TreePath>, out: &mut Vec<Declaration>) -> ParseResult<()> {
        if self.peek_is(Token::BraceLeft) {
            return self.parse_braced_block(prefix, out);
        }

        self.expect_next(Token::Indent)?;

        self.skip_declaration_delimiters();
        while !self.peek_is(Token::Dedent) && !self.peek_is(Token::Eof) {
            self.parse_declaration(prefix, out)?;
            self.skip_declaration_delimiters();
        }

        self.expect_next(Token::Dedent)?;

        Ok(())
    }

    fn parse_braced_block(&mut self, prefix: Option<&TreePath>, out: &mut Vec<Declaration>) -> ParseResult<()> {
        self.expect_next(Token::BraceLeft)?;

        self.skip_declaration_delimiters();
        let indented = self.consume(Token::Indent);
        self.skip_declaration_delimiters();
        while !self.peek_is(Token::BraceRight) && !self.peek_is(Token::Eof) {
            if indented && self.consume(Token::Dedent) {
                self.skip_declaration_delimiters();
                break;
            }

            self.parse_declaration(prefix, out)?;
            self.skip_declaration_delimiters();
        }

        if indented {
            self.consume(Token::Dedent);
        }

        self.expect_next(Token::BraceRight)?;

        Ok(())
    }

    fn parse_optional_declaration_block(&mut self) -> ParseResult<Vec<Declaration>> {
        let mut declarations = Vec::new();
        if self.at_declaration_block() {
            self.skip_newlines();
            self.parse_block(None, &mut declarations)?;
        }

        Ok(declarations)
    }

    fn at_declaration_block(&mut self) -> bool {
        let checkpoint = self.cursor;
        self.skip_newlines();
        let open = self.peek_is(Token::Indent) || self.peek_is(Token::BraceLeft);
        self.cursor = checkpoint;

        open
    }

    fn at_indented_block(&mut self) -> bool {
        let checkpoint = self.cursor;
        self.skip_newlines();
        let indented = self.peek_is(Token::Indent);
        self.cursor = checkpoint;

        indented
    }

    /// `list(a = /datum/foo.)`
    fn skip_dangling_access(&mut self) -> bool {
        let dangling = matches!(self.peek(), Some((Token::Dot | Token::SafeDot, _)))
            && self.peek_at(1).is_none_or(|(token, _)| token.word().is_none());

        if dangling {
            self.cursor += 1;
        }

        dangling
    }

    /// `/obj/item/ in contents` is accepted as `/obj/item in contents` by BYOND.
    fn skip_dangling_path_separator(&mut self) -> bool {
        let dangling = self.peek_is(Token::Slash) && self.peek_at_is(1, Token::In) && !self.token_touches_next(0);

        if dangling {
            self.cursor += 1;
        }

        dangling
    }

    /// `/mob.proc/Life`, `.proc/Life`, `/mob/proc/Life`, `proc/Life`
    fn at_path_dot(&self) -> bool {
        self.peek_is(Token::Dot)
            && (self.peek_at_is(1, Token::Soft(SoftKeyword::Proc))
                || self.peek_at_is(1, Token::Soft(SoftKeyword::Verb)))
    }

    /// `var/matrix/final` declares `final` and `var/datum/admin_verb/verb` declares `verb`
    fn keyword_ends_path(&mut self) -> bool {
        if self.peek_at_is(1, Token::Slash) {
            return false;
        }

        let checkpoint = self.cursor;
        self.cursor += 1;
        let block = self.at_declaration_block();
        self.cursor = checkpoint;

        !block
    }

    fn at_operator_proc_name(&mut self) -> bool {
        let checkpoint = self.cursor;
        self.cursor += 1;
        let operator_proc = self.parse_operator_name().is_ok() && self.peek_is(Token::ParenLeft);
        self.cursor = checkpoint;

        operator_proc
    }

    fn parse_path(&mut self) -> ParseResult<TreePath> {
        let absolute = self.peek_is(Token::Slash);
        // `/datum/manipulator_task/cargo/dropoff_base/throw`
        let mut after_separator = absolute || self.at_path_dot();
        if after_separator {
            self.advance()?;
        }

        let mut segments = Vec::new();
        let mut flags = PathFlags::NONE;
        let mut keyword_offset = None;

        loop {
            let (token, location) = self.peek().ok_or_else(ParseError::end_of_file)?;
            match token {
                Token::Identifier(s) => {
                    self.advance()?;
                    segments.push(Identifier::from(s.to_string()));
                },
                Token::Soft(SoftKeyword::Operator) if self.at_operator_proc_name() => {
                    self.advance()?;
                    flags |= PathFlags::IS_PROC | PathFlags::IS_OPERATOR;
                    keyword_offset.get_or_insert(segments.len());
                    segments.push(Identifier::from(self.parse_operator_name()?));
                    break;
                },
                Token::Soft(keyword) => {
                    let modifier =
                        path_modifier(keyword).filter(|modifier| !(modifier.may_be_name && self.keyword_ends_path()));
                    self.advance()?;
                    match modifier {
                        Some(modifier) => {
                            flags |= modifier.flags;
                            if modifier.declares {
                                keyword_offset.get_or_insert(segments.len());
                            }
                        },
                        None => segments.push(Identifier::from(keyword.as_word().to_string())),
                    }
                },
                _ => {
                    let Some(word) = token.word().filter(|_| after_separator) else {
                        if segments.is_empty() && flags.is_empty() {
                            return Err(ParseError::expected_path(location));
                        }

                        break;
                    };
                    self.advance()?;
                    segments.push(Identifier::from(word.to_string()));
                },
            }

            if self.skip_dangling_path_separator() {
                break;
            } else if self.peek_is(Token::Slash) || self.at_path_dot() {
                self.advance()?;
                after_separator = true;
            } else if segments.is_empty()
                && flags.intersects(PathFlags::KEYWORD)
                && self.peek().is_some_and(|(token, _)| token.is_identifier())
            {
                // `var response = Bridge(...)`
                after_separator = true;
            } else {
                break;
            }
        }

        if !flags.contains(PathFlags::IS_PROC)
            && !flags.contains(PathFlags::IS_VERB)
            && !flags.contains(PathFlags::IS_VAR)
        {
            flags |= PathFlags::IS_DATUM;
        }

        let name_offset = segments.len().saturating_sub(1);
        Ok(TreePath {
            keyword_offset: keyword_offset.unwrap_or(segments.len()),
            segments,
            absolute,
            flags,
            name_offset,
        })
    }

    fn parse_operator_name(&mut self) -> ParseResult<String> {
        if self.consume(Token::SquareLeft) {
            self.expect_next(Token::SquareRight)?;
            let mut name = String::from("operator[]");
            if self.consume(Token::Equal) {
                name.push('=');
            }
            return Ok(name);
        }

        if self.consume(Token::ParenLeft) {
            self.expect_next(Token::ParenRight)?;
            return Ok(String::from("operator()"));
        }

        if self.consume(Token::StringLiteral("")) {
            return Ok(String::from("operator\"\""));
        }

        let (token, location) = self.advance()?;
        if matches!(
            token,
            Token::Add
                | Token::Sub
                | Token::Mul
                | Token::Slash
                | Token::Modulo
                | Token::FloatModulo
                | Token::Pow
                | Token::BitAnd
                | Token::BitOr
                | Token::BitXor
                | Token::BitNot
                | Token::ShiftLeft
                | Token::ShiftRight
                | Token::CompareEqual
                | Token::CompareNotEqual
                | Token::CompareNotEqualAlt
                | Token::CompareThreeWay
                | Token::Equivalent
                | Token::NotEquivalent
                | Token::AngleLeft
                | Token::AngleRight
                | Token::LessEqual
                | Token::GreaterEqual
                | Token::Increment
                | Token::Decrement
                | Token::AssignInto
                | Token::AddEqual
                | Token::SubEqual
                | Token::MulEqual
                | Token::DivEqual
                | Token::ModEqual
                | Token::FloatModuloEqual
                | Token::PowEqual
                | Token::AndEqual
                | Token::OrEqual
                | Token::XorEqual
                | Token::ShiftLeftEqual
                | Token::ShiftRightEqual
        ) {
            Ok(format!("operator{token}"))
        } else {
            Err(ParseError::unexpected("an overloadable operator", token, location))
        }
    }

    /// `var { name }`
    fn parse_declaration(&mut self, prefix: Option<&TreePath>, out: &mut Vec<Declaration>) -> ParseResult<()> {
        let (_, location) = self.peek().ok_or_else(ParseError::end_of_file)?;
        let in_var_block = prefix.is_some_and(|prefix| prefix.flags.contains(PathFlags::IS_VAR));

        if !in_var_block
            && let (Some((token, _)), Some((Token::Equal, _))) = (self.peek(), self.peek_at(1))
            && let Some(name) = token.identifier_name()
        {
            self.advance()?;
            self.advance()?;
            let value = self.parse_expression(Precedence::Lowest)?;

            out.push(Declaration::Override {
                name: Identifier::from(name.to_string()),
                value,
                location,
            });

            return Ok(());
        }

        let parsed = self.parse_path()?;
        let path = match prefix {
            Some(prefix) => prefix.concat(&parsed),
            None => parsed,
        };

        let is_proc = path.flags.intersects(PathFlags::IS_PROC | PathFlags::IS_VERB);
        if is_proc && self.peek_is(Token::ParenLeft) {
            out.push(self.parse_proc(path, location)?);

            return Ok(());
        }

        if path.flags.intersects(PathFlags::KEYWORD) && self.at_declaration_block() {
            return self.parse_keyword_block(&path, out);
        }

        if path.flags.contains(PathFlags::IS_VAR) {
            return self.parse_var(path, location, out);
        }

        if is_proc || self.peek_is(Token::ParenLeft) {
            out.push(self.parse_proc(path, location)?);

            return Ok(());
        }

        // `/datum/admin_verb/atmos_debug/visibility_flag = FLAG`
        if self.consume(Token::Equal) {
            let initializer = Some(self.parse_expression(Precedence::Assignment)?);
            out.push(Declaration::Var {
                path,
                var_type: None,
                modifiers: VarModifiers::default(),
                dimensions: Vec::new(),
                as_type: None,
                in_list: None,
                initializer,
                location,
            });

            return Ok(());
        }

        let body = self.parse_optional_declaration_block()?;
        out.push(Declaration::Type { path, body, location });

        Ok(())
    }

    /// `var/const { NORTH = 1 }`
    fn parse_keyword_block(&mut self, prefix: &TreePath, out: &mut Vec<Declaration>) -> ParseResult<()> {
        self.skip_newlines();

        self.parse_block(Some(prefix), out)
    }

    fn parse_var(&mut self, path: TreePath, location: Location, out: &mut Vec<Declaration>) -> ParseResult<()> {
        let mut path = path;
        let mut location = location;

        loop {
            let mut spec = self.var_spec_from_path(&path);
            self.parse_var_spec_suffix(&mut spec)?;

            let initializer = if self.consume(Token::Equal) {
                Some(self.parse_expression(Precedence::Assignment)?)
            } else {
                None
            };

            if self.consume(Token::As) {
                spec.as_type = Some(self.parse_type_spec()?);
            }

            let in_list = if self.consume(Token::In) {
                Some(self.parse_range_expression()?)
            } else {
                None
            };

            let next = if self.consume(Token::Comma) && !self.at_statement_end() {
                let (_, next_location) = self.peek().ok_or_else(ParseError::end_of_file)?;
                let parsed = self.parse_path()?;

                Some((path.sibling(&parsed), next_location))
            } else {
                None
            };

            out.push(Declaration::Var {
                path,
                var_type: spec.var_type,
                modifiers: spec.modifiers,
                dimensions: spec.dimensions,
                as_type: spec.as_type,
                in_list,
                initializer,
                location,
            });

            match next {
                Some((next_path, next_location)) => {
                    path = next_path;
                    location = next_location;
                },
                None => return Ok(()),
            }
        }
    }

    fn var_spec_from_path(&self, path: &TreePath) -> VarSpec {
        let declared = if path.flags.contains(PathFlags::IS_VAR) {
            path.declared_type()
        } else {
            &path.segments[..path.name_offset.min(path.segments.len())]
        };
        VarSpec {
            name: path.name().cloned().unwrap_or_else(|| Identifier::from(String::new())),
            var_type: (!declared.is_empty()).then(|| TreePath::new(declared.to_vec(), true)),
            modifiers: VarModifiers {
                is_const: path.flags.contains(PathFlags::IS_CONST),
                is_final: path.flags.contains(PathFlags::IS_FINAL),
                is_global: path.flags.contains(PathFlags::IS_GLOBAL),
                is_static: path.flags.contains(PathFlags::IS_STATIC),
                is_tmp: path.flags.contains(PathFlags::IS_TMP),
            },
            dimensions: Vec::new(),
            as_type: None,
        }
    }

    fn parse_var_spec_suffix(&mut self, spec: &mut VarSpec) -> ParseResult<()> {
        while self.consume(Token::SquareLeft) {
            let size = if self.peek_is(Token::SquareRight) {
                None
            } else {
                Some(self.parse_expression(Precedence::Lowest)?)
            };
            self.expect_next(Token::SquareRight)?;
            spec.dimensions.push(size);
        }

        if self.consume(Token::As) {
            spec.as_type = Some(self.parse_type_spec()?);
        }

        Ok(())
    }

    fn parse_proc(&mut self, path: TreePath, location: Location) -> ParseResult<Declaration> {
        let kind = if path.flags.contains(PathFlags::IS_OPERATOR) {
            ProcKind::Operator
        } else if path.flags.contains(PathFlags::IS_VERB) {
            ProcKind::Verb
        } else if path.flags.contains(PathFlags::IS_PROC) {
            ProcKind::Proc
        } else {
            ProcKind::Override
        };

        let (params, variadic) = self.parse_proc_params()?;
        let return_type = if self.consume(Token::As) {
            Some(self.parse_type_spec()?)
        } else {
            None
        };
        let body = self.parse_optional_statement_block()?;

        Ok(Declaration::Proc {
            path,
            kind,
            params,
            variadic,
            return_type,
            body,
            location,
        })
    }

    fn parse_proc_params(&mut self) -> ParseResult<(Vec<ProcParam>, bool)> {
        self.expect_next(Token::ParenLeft)?;
        let mut params = Vec::new();
        let mut variadic = false;

        while !self.peek_is(Token::ParenRight) {
            // `New(..., serialized_data)`
            if self.consume(Token::Ellipsis) {
                variadic = true;
                if !self.consume(Token::Comma) {
                    break;
                }

                continue;
            }

            // BYOND permits `null` as a formal name, where it shadows the keyword.
            let path = if self.consume(Token::Null) {
                TreePath::new(vec![Identifier::from("null".to_string())], false)
            } else {
                self.parse_path()?
            };
            let mut spec = self.var_spec_from_path(&path);
            self.parse_var_spec_suffix(&mut spec)?;
            let default = if self.consume(Token::Equal) {
                Some(self.parse_expression(Precedence::Assignment)?)
            } else {
                None
            };

            if self.consume(Token::As) {
                spec.as_type = Some(self.parse_type_spec()?);
            }

            let in_list = if self.consume(Token::In) {
                Some(self.parse_range_expression()?)
            } else {
                None
            };
            params.push(ProcParam { spec, default, in_list });

            if !self.consume(Token::Comma) {
                break;
            }
        }

        self.expect_next(Token::ParenRight)?;

        Ok((params, variadic))
    }

    fn parse_type_spec(&mut self) -> ParseResult<TypeSpec> {
        let parenthesized = self.consume(Token::ParenLeft);
        let mut spec = TypeSpec::default();

        loop {
            let (token, location) = self.peek().ok_or_else(ParseError::end_of_file)?;
            let flag = match token {
                Token::Null => Some(InputType::NULL),
                token => token.identifier_name().and_then(InputType::from_word),
            };

            if let Some(flag) = flag {
                self.advance()?;
                spec.flags |= flag;
            } else if token == Token::Slash || token.is_identifier() {
                if spec.path.is_some() {
                    return Err(ParseError::unexpected("a type flag", token, location));
                }
                spec.path = Some(self.parse_path()?);
            } else {
                return Err(ParseError::unexpected("an input type or path", token, location));
            }

            if !self.consume(Token::BitOr) {
                break;
            }
        }

        if parenthesized {
            self.expect_next(Token::ParenRight)?;
        }

        Ok(spec)
    }

    fn parse_optional_statement_block(&mut self) -> ParseResult<Option<Vec<Statement>>> {
        if self.peek_is(Token::Indent) {
            return self.parse_statement_block().map(Some);
        }

        if self.peek_is(Token::BraceLeft) {
            return self.parse_braced_statement_block().map(Some);
        }

        // `Multiply(m) return matrix(...)`
        if !self.at_statement_end() {
            let mut statements = Vec::new();
            while !self.at_statement_end() {
                statements.extend(self.parse_statement()?);
                if !self.consume(Token::Semicolon) {
                    break;
                }
            }

            return Ok(Some(statements));
        }

        let checkpoint = self.cursor;
        self.skip_newlines();
        if self.peek_is(Token::Indent) {
            self.parse_statement_block().map(Some)
        } else if self.peek_is(Token::BraceLeft) {
            self.parse_braced_statement_block().map(Some)
        } else {
            self.cursor = checkpoint;
            Ok(None)
        }
    }

    fn parse_statement_block(&mut self) -> ParseResult<Vec<Statement>> {
        self.expect_next(Token::Indent)?;
        let mut statements = Vec::new();
        self.skip_statement_delimiters();
        while !self.peek_is(Token::Dedent) && !self.peek_is(Token::Eof) {
            statements.extend(self.parse_statement()?);
            self.skip_statement_delimiters();
        }

        self.expect_next(Token::Dedent)?;

        Ok(statements)
    }

    fn parse_braced_statement_block(&mut self) -> ParseResult<Vec<Statement>> {
        self.expect_next(Token::BraceLeft)?;
        self.skip_statement_delimiters();

        let indented = self.consume(Token::Indent);
        let mut statements = Vec::new();
        self.skip_statement_delimiters();

        while !self.peek_is(Token::BraceRight) && !self.peek_is(Token::Eof) {
            if indented && self.consume(Token::Dedent) {
                self.skip_statement_delimiters();
                break;
            }

            statements.extend(self.parse_statement()?);
            self.skip_statement_delimiters();
        }

        if indented {
            self.consume(Token::Dedent);
        }

        self.expect_next(Token::BraceRight)?;

        Ok(statements)
    }

    fn parse_required_body(&mut self) -> ParseResult<Vec<Statement>> {
        if self.peek_is(Token::BraceLeft) {
            return self.parse_braced_statement_block();
        }

        if self.peek_is(Token::Indent) {
            return self.parse_statement_block();
        }

        if self.peek_is(Token::Newline) {
            let checkpoint = self.cursor;
            self.skip_newlines();
            if self.peek_is(Token::Indent) {
                return self.parse_statement_block();
            }

            if self.peek_is(Token::BraceLeft) {
                return self.parse_braced_statement_block();
            }
            self.cursor = checkpoint;

            return Ok(Vec::new());
        }

        if self.at_statement_end() {
            return Ok(Vec::new());
        }

        self.parse_statement()
    }

    fn parse_statement(&mut self) -> ParseResult<Vec<Statement>> {
        let (statements, needs_terminator) = match self.peek().ok_or_else(ParseError::end_of_file)?.0 {
            Token::Semicolon => {
                self.advance()?;
                (vec![Statement::Empty], false)
            },
            Token::Return => (vec![self.parse_return_statement()?], true),
            Token::If => (vec![self.parse_if_statement()?], false),
            Token::While => (vec![self.parse_while_statement()?], false),
            Token::Do => (vec![self.parse_do_while_statement()?], true),
            Token::For => (vec![self.parse_for_statement()?], false),
            Token::Switch => (vec![self.parse_switch_statement()?], false),
            Token::Spawn => (vec![self.parse_spawn_statement()?], false),
            Token::Try => (vec![self.parse_try_catch_statement()?], false),
            Token::Throw => {
                self.advance()?;
                (vec![Statement::Throw(self.parse_expression(Precedence::Lowest)?)], true)
            },
            Token::Del => {
                self.advance()?;
                (vec![Statement::Del(self.parse_expression(Precedence::Lowest)?)], true)
            },
            Token::Set => (self.parse_setting_statements()?, true),
            Token::Break => {
                self.advance()?;
                (vec![Statement::Break(self.parse_optional_identifier()?)], true)
            },
            Token::Continue => {
                self.advance()?;
                (vec![Statement::Continue(self.parse_optional_identifier()?)], true)
            },
            Token::Goto => {
                self.advance()?;
                (vec![Statement::Goto(self.parse_identifier()?)], true)
            },
            Token::Soft(SoftKeyword::Var) | Token::Slash if self.starts_var_path() => {
                let mut vars = Vec::new();
                let block = self.parse_var_statements(None, &mut vars)?;

                (vars, !block)
            },
            token if token.is_identifier() && self.peek_at_is(1, Token::Colon) && !self.token_touches_next(1) => {
                self.advance()?;
                self.advance()?;
                let body = self.parse_optional_statement_block()?.unwrap_or_default();
                (
                    vec![Statement::Label {
                        name: Identifier::from(token.identifier_name().unwrap_or_default().to_string()),
                        body,
                    }],
                    false,
                )
            },
            _ => (vec![self.parse_expression_statement()?], true),
        };

        if needs_terminator {
            self.ensure_statement_end()?;
        }

        Ok(statements)
    }

    fn ensure_statement_end(&self) -> ParseResult<()> {
        if self.at_statement_end() {
            return Ok(());
        }

        let (token, location) = self.peek().ok_or_else(ParseError::end_of_file)?;

        Err(ParseError::unexpected("the end of a statement", token, location))
    }

    fn starts_var_path(&self) -> bool {
        self.peek_is(Token::Soft(SoftKeyword::Var))
            || (self.peek_is(Token::Slash) && self.peek_at_is(1, Token::Soft(SoftKeyword::Var)))
    }

    fn parse_optional_identifier(&mut self) -> ParseResult<Option<Identifier>> {
        if self.at_statement_end() {
            Ok(None)
        } else {
            self.parse_identifier().map(Some)
        }
    }

    fn parse_identifier(&mut self) -> ParseResult<Identifier> {
        let (token, location) = self.advance()?;
        token
            .word()
            .map(|name| Identifier::from(name.to_string()))
            .ok_or_else(|| ParseError::unexpected("an identifier", token, location))
    }

    fn parse_return_statement(&mut self) -> ParseResult<Statement> {
        self.expect_next(Token::Return)?;
        let value = if self.at_statement_end() {
            None
        } else {
            Some(self.parse_expression(Precedence::Lowest)?)
        };

        Ok(Statement::Return(value))
    }

    fn parse_parenthesized_expression(&mut self) -> ParseResult<ExpressionId> {
        self.expect_next(Token::ParenLeft)?;
        let expression = self.parse_expression(Precedence::Lowest)?;
        self.expect_next(Token::ParenRight)?;
        Ok(expression)
    }

    fn parse_if_statement(&mut self) -> ParseResult<Statement> {
        self.expect_next(Token::If)?;
        let condition = self.parse_parenthesized_expression()?;
        let body = self.parse_required_body()?;
        let mut branches = vec![(condition, body)];
        let mut else_branch = None;

        let checkpoint = self.cursor;
        self.skip_statement_delimiters();
        if self.consume(Token::Else) {
            if self.peek_is(Token::If) {
                let Statement::If {
                    branches: more,
                    else_branch: tail,
                } = self.parse_if_statement()?
                else {
                    unreachable!()
                };
                branches.extend(more);
                else_branch = tail;
            } else {
                else_branch = Some(self.parse_required_body()?);
            }
        } else {
            self.cursor = checkpoint;
        }

        Ok(Statement::If { branches, else_branch })
    }

    fn parse_while_statement(&mut self) -> ParseResult<Statement> {
        self.expect_next(Token::While)?;
        let condition = self.parse_parenthesized_expression()?;
        let body = self.parse_required_body()?;

        Ok(Statement::While { condition, body })
    }

    fn parse_do_while_statement(&mut self) -> ParseResult<Statement> {
        self.expect_next(Token::Do)?;
        let body = self.parse_required_body()?;
        self.skip_statement_delimiters();
        self.expect_next(Token::While)?;
        let condition = self.parse_parenthesized_expression()?;

        Ok(Statement::DoWhile { body, condition })
    }

    fn parse_for_statement(&mut self) -> ParseResult<Statement> {
        self.expect_next(Token::For)?;
        self.expect_next(Token::ParenLeft)?;

        if self.consume(Token::ParenRight) {
            return self.finish_standard_for(None, None, None);
        }

        if self.consume(Token::Comma) || self.consume(Token::Semicolon) {
            return self.parse_standard_for_tail_after_separator(None);
        }

        if self.starts_var_path() {
            let binding = self.parse_loop_binding()?;
            let spec = binding.spec.clone();

            if self.consume(Token::Equal) {
                let start = self.parse_expression(Precedence::Assignment)?;
                if self.consume(Token::To) {
                    return self.finish_range_for(binding, start);
                }

                let init = Statement::Var {
                    spec,
                    initializer: Some(start),
                    in_list: None,
                };
                return self.parse_standard_for_tail(Some(init));
            }

            if self.consume(Token::In) {
                return self.parse_for_membership(None, binding);
            }

            if self.consume(Token::Comma) {
                let checkpoint = self.cursor;
                if self.can_start_loop_binding()
                    && let Ok(value) = self.parse_loop_binding()
                    && self.consume(Token::In)
                {
                    return self.parse_for_membership(Some(binding), value);
                }

                self.cursor = checkpoint;
                let init = Statement::Var {
                    spec,
                    initializer: None,
                    in_list: None,
                };
                return self.parse_standard_for_tail_after_separator(Some(init));
            }

            if self.consume(Token::Semicolon) {
                let init = Statement::Var {
                    spec,
                    initializer: None,
                    in_list: None,
                };
                return self.parse_standard_for_tail_after_separator(Some(init));
            }

            if self.consume(Token::ParenRight) {
                return self.finish_list_for(None, binding, None);
            }

            let (token, location) = self.peek().ok_or_else(ParseError::end_of_file)?;
            return Err(ParseError::unexpected("in, =, a separator, or )", token, location));
        }

        let first = self.parse_expression(Precedence::Assignment)?;
        // `for(point as anything in grid)`
        let as_type = self.consume(Token::As).then(|| self.parse_type_spec()).transpose()?;
        if as_type.is_none() && self.consume(Token::To) {
            let (binding, start) = self.assignment_loop_binding(first)?;
            return self.finish_range_for(binding, start);
        }

        if self.consume(Token::In) {
            let mut binding = self.loop_binding_from_expression(first)?;
            binding.spec.as_type = as_type;
            return self.parse_for_membership(None, binding);
        }

        if self.consume(Token::Comma) {
            let checkpoint = self.cursor;
            if self.can_start_loop_binding()
                && let Ok(value) = self.parse_loop_binding()
                && self.consume(Token::In)
            {
                let key = self.loop_binding_from_expression(first)?;
                return self.parse_for_membership(Some(key), value);
            }

            self.cursor = checkpoint;
            return self.parse_standard_for_tail_after_separator(Some(Statement::Expression(first)));
        }

        if self.consume(Token::Semicolon) {
            return self.parse_standard_for_tail_after_separator(Some(Statement::Expression(first)));
        }

        if self.consume(Token::ParenRight) {
            let binding = self.loop_binding_from_expression(first)?;
            return self.finish_list_for(None, binding, None);
        }

        let (token, location) = self.peek().ok_or_else(ParseError::end_of_file)?;
        Err(ParseError::unexpected("in, to, a separator, or )", token, location))
    }

    fn parse_standard_for_tail(&mut self, init: Option<Statement>) -> ParseResult<Statement> {
        if !(self.consume(Token::Comma) || self.consume(Token::Semicolon)) {
            let (token, location) = self.advance()?;
            return Err(ParseError::unexpected("a for-loop separator", token, location));
        }

        self.parse_standard_for_tail_after_separator(init)
    }

    fn parse_standard_for_tail_after_separator(&mut self, init: Option<Statement>) -> ParseResult<Statement> {
        let condition =
            if self.peek_is(Token::Comma) || self.peek_is(Token::Semicolon) || self.peek_is(Token::ParenRight) {
                None
            } else {
                Some(self.parse_expression(Precedence::Lowest)?)
            };

        if self.consume(Token::ParenRight) {
            return self.finish_standard_for(init, condition, None);
        }

        if !(self.consume(Token::Comma) || self.consume(Token::Semicolon)) {
            let (token, location) = self.advance()?;
            return Err(ParseError::unexpected("a for-loop separator", token, location));
        }

        let step = if self.peek_is(Token::ParenRight) {
            None
        } else {
            Some(Box::new(Statement::Expression(
                self.parse_expression(Precedence::Lowest)?,
            )))
        };

        self.expect_next(Token::ParenRight)?;

        self.finish_standard_for(init, condition, step)
    }

    fn finish_standard_for(
        &mut self, init: Option<Statement>, condition: Option<ExpressionId>, step: Option<Box<Statement>>,
    ) -> ParseResult<Statement> {
        let body = self.parse_required_body()?;

        Ok(Statement::For(Box::new(ForLoop::Standard {
            init: init.map(Box::new),
            condition,
            step,
            body,
        })))
    }

    fn can_start_loop_binding(&self) -> bool {
        self.starts_var_path() || self.peek().is_some_and(|(token, _)| token.is_identifier())
    }

    fn parse_loop_binding(&mut self) -> ParseResult<LoopBinding> {
        let declares = self.starts_var_path();
        let path = self.parse_path()?;
        let mut spec = self.var_spec_from_path(&path);
        self.parse_var_spec_suffix(&mut spec)?;

        Ok(LoopBinding { spec, declares })
    }

    fn loop_binding_from_expression(&self, expression: ExpressionId) -> ParseResult<LoopBinding> {
        let name = self.expression_identifier(expression)?;
        Ok(LoopBinding {
            spec: VarSpec {
                name,
                var_type: None,
                modifiers: VarModifiers::default(),
                dimensions: Vec::new(),
                as_type: None,
            },
            declares: false,
        })
    }

    fn assignment_loop_binding(&self, expression: ExpressionId) -> ParseResult<(LoopBinding, ExpressionId)> {
        match self.expressions.get(expression.index()) {
            Some(Expression::Assign {
                kind: AssignmentKind::Assign,
                lhs_expr,
                rhs_expr,
            }) => Ok((self.loop_binding_from_expression(*lhs_expr)?, *rhs_expr)),
            _ => Err(ParseError::expected_expression(Location::default())),
        }
    }

    fn parse_for_membership(&mut self, key: Option<LoopBinding>, value: LoopBinding) -> ParseResult<Statement> {
        if self.consume(Token::ParenRight) {
            return self.finish_list_for(key, value, None);
        }

        let list_or_start = self.parse_expression(Precedence::Assignment)?;
        if self.consume(Token::To) {
            if key.is_some() {
                let (token, location) = self.peek().ok_or_else(ParseError::end_of_file)?;
                return Err(ParseError::unexpected("a list expression", token, location));
            }

            return self.finish_range_for(value, list_or_start);
        }

        self.expect_next(Token::ParenRight)?;
        self.finish_list_for(key, value, Some(list_or_start))
    }

    fn finish_list_for(
        &mut self, key: Option<LoopBinding>, value: LoopBinding, list: Option<ExpressionId>,
    ) -> ParseResult<Statement> {
        let body = self.parse_required_body()?;

        Ok(Statement::For(Box::new(ForLoop::List { key, value, list, body })))
    }

    fn finish_range_for(&mut self, variable: LoopBinding, start: ExpressionId) -> ParseResult<Statement> {
        let end = self.parse_expression(Precedence::Assignment)?;
        let step = if self.consume_step() {
            Some(self.parse_expression(Precedence::Assignment)?)
        } else {
            None
        };
        self.expect_next(Token::ParenRight)?;
        let body = self.parse_required_body()?;

        Ok(Statement::For(Box::new(ForLoop::Range {
            variable,
            start,
            end,
            step,
            body,
        })))
    }

    fn expression_identifier(&self, expression: ExpressionId) -> ParseResult<Identifier> {
        match self.expressions.get(expression.index()) {
            Some(Expression::Identifier(name)) => Ok(name.clone()),
            Some(Expression::Grouped(inner)) => self.expression_identifier(*inner),
            _ => Err(ParseError::expected_expression(Location::default())),
        }
    }

    fn parse_switch_statement(&mut self) -> ParseResult<Statement> {
        self.expect_next(Token::Switch)?;
        let value = self.parse_parenthesized_expression()?;
        self.skip_statement_delimiters();

        let braced = self.consume(Token::BraceLeft);
        if !braced {
            self.expect_next(Token::Indent)?;
        } else {
            self.skip_statement_delimiters();
        }

        let brace_indent = braced && self.consume(Token::Indent);
        self.skip_statement_delimiters();

        let mut cases = Vec::new();
        let mut default = None;
        loop {
            if braced && self.peek_is(Token::BraceRight) {
                break;
            }

            if self.peek_is(Token::Dedent) {
                self.advance()?;
                break;
            }

            if self.peek_is(Token::Eof) {
                return Err(ParseError::end_of_file());
            }

            if self.consume(Token::If) {
                self.expect_next(Token::ParenLeft)?;
                let mut values = Vec::new();
                while !self.peek_is(Token::ParenRight) {
                    let start = self.parse_expression(Precedence::Assignment)?;
                    if self.consume(Token::To) {
                        let end = self.parse_expression(Precedence::Assignment)?;
                        values.push(SwitchValue::Range(start, end));
                    } else {
                        values.push(SwitchValue::Value(start));
                    }

                    if !self.consume(Token::Comma) {
                        break;
                    }
                }

                self.expect_next(Token::ParenRight)?;
                let body = self.parse_required_body()?;
                cases.push(SwitchCase { values, body });
            } else if self.consume(Token::Else) {
                default = Some(self.parse_required_body()?);
            } else {
                let (token, location) = self.advance()?;
                return Err(ParseError::unexpected("if or else in switch", token, location));
            }

            self.skip_statement_delimiters();
        }

        if braced {
            if brace_indent {
                self.consume(Token::Dedent);
            }

            self.skip_statement_delimiters();
            self.expect_next(Token::BraceRight)?;
        }

        Ok(Statement::Switch { value, cases, default })
    }

    fn parse_spawn_statement(&mut self) -> ParseResult<Statement> {
        self.expect_next(Token::Spawn)?;
        let delay = if self.consume(Token::ParenLeft) {
            let delay = if self.peek_is(Token::ParenRight) {
                None
            } else {
                Some(self.parse_expression(Precedence::Lowest)?)
            };
            self.expect_next(Token::ParenRight)?;
            delay
        } else {
            None
        };
        let body = self.parse_required_body()?;

        Ok(Statement::Spawn { delay, body })
    }

    fn parse_try_catch_statement(&mut self) -> ParseResult<Statement> {
        self.expect_next(Token::Try)?;
        let try_body = self.parse_required_body()?;
        let checkpoint = self.cursor;
        self.skip_statement_delimiters();
        if !self.consume(Token::Catch) {
            self.cursor = checkpoint;
            let (token, location) = self.advance()?;
            return Err(ParseError::unexpected("catch", token, location));
        }

        let catch_param = if self.consume(Token::ParenLeft) {
            if self.consume(Token::ParenRight) {
                None
            } else {
                let path = self.parse_path()?;
                let mut spec = self.var_spec_from_path(&path);
                self.parse_var_spec_suffix(&mut spec)?;
                self.expect_next(Token::ParenRight)?;
                Some(spec)
            }
        } else {
            None
        };
        let catch_body = self.parse_required_body()?;

        Ok(Statement::TryCatch {
            try_body,
            catch_param,
            catch_body,
        })
    }

    fn parse_setting_statements(&mut self) -> ParseResult<Vec<Statement>> {
        self.expect_next(Token::Set)?;
        let mut settings = Vec::new();
        loop {
            let name = self.parse_identifier()?;
            let mode = if self.consume(Token::Equal) {
                SettingMode::Assign
            } else if self.consume(Token::In) {
                SettingMode::In
            } else {
                let (token, location) = self.advance()?;
                return Err(ParseError::unexpected("= or in", token, location));
            };
            let value = self.parse_expression(Precedence::Lowest)?;
            settings.push(Statement::Setting { name, mode, value });

            if !self.consume(Token::Comma) {
                break;
            }
        }

        Ok(settings)
    }

    fn parse_var_statements(&mut self, prefix: Option<&TreePath>, out: &mut Vec<Statement>) -> ParseResult<bool> {
        loop {
            let parsed = self.parse_path()?;
            let path = match prefix {
                Some(prefix) => prefix.concat(&parsed),
                None => parsed,
            };

            if self.at_indented_block() {
                self.skip_newlines();
                self.expect_next(Token::Indent)?;
                self.skip_statement_delimiters();
                while !self.peek_is(Token::Dedent) && !self.peek_is(Token::Eof) {
                    self.parse_var_statements(Some(&path), out)?;
                    self.skip_statement_delimiters();
                }

                self.expect_next(Token::Dedent)?;

                return Ok(true);
            }

            let mut spec = self.var_spec_from_path(&path);
            self.parse_var_spec_suffix(&mut spec)?;
            // `var/found = locate(/obj/tray) in loc || locate(/obj/plate) in loc`
            let initializer = if self.consume(Token::Equal) {
                Some(self.parse_expression(Precedence::Lowest)?)
            } else {
                None
            };

            if self.consume(Token::As) {
                spec.as_type = Some(self.parse_type_spec()?);
            }

            let in_list = if initializer.is_none() && self.consume(Token::In) {
                Some(self.parse_range_expression()?)
            } else {
                None
            };
            out.push(Statement::Var {
                spec,
                initializer,
                in_list,
            });

            if !self.consume(Token::Comma) {
                break;
            }

            if self.at_statement_end() {
                break;
            }
        }

        Ok(false)
    }

    fn parse_expression_statement(&mut self) -> ParseResult<Statement> {
        let expression = self.parse_expression(Precedence::Lowest)?;
        match self.expressions.get(expression.index()) {
            Some(Expression::Binary {
                op: BinaryOp::ShiftLeft,
                lhs_expr,
                rhs_expr,
            }) => Ok(Statement::Output {
                target: *lhs_expr,
                value: *rhs_expr,
            }),
            Some(Expression::Binary {
                op: BinaryOp::ShiftRight,
                lhs_expr,
                rhs_expr,
            }) => Ok(Statement::Input {
                source: *lhs_expr,
                destination: *rhs_expr,
            }),
            _ => Ok(Statement::Expression(expression)),
        }
    }

    fn parse_range_expression(&mut self) -> ParseResult<ExpressionId> {
        let start = self.parse_expression(Precedence::Assignment)?;
        if !self.consume(Token::To) {
            return Ok(start);
        }

        let end = self.parse_expression(Precedence::Assignment)?;
        let step = if self.consume_step() {
            Some(self.parse_expression(Precedence::Assignment)?)
        } else {
            None
        };

        Ok(self.make_expr(Expression::Range { start, end, step }))
    }

    fn parse_expression(&mut self, precedence: Precedence) -> ParseResult<ExpressionId> {
        match precedence {
            Precedence::Lowest => self.parse_expression(Precedence::In),
            Precedence::In => self.parse_in_expression(),
            Precedence::Assignment => self.parse_assignment_expression(),
            Precedence::Ternary => self.parse_ternary_expression(),
            Precedence::Unary => self.parse_unary_expression(),
            Precedence::Postfix => self.parse_postfix_expression(),
            _ => self.parse_binary_expression(precedence),
        }
    }

    fn parse_in_expression(&mut self) -> ParseResult<ExpressionId> {
        let mut value = self.parse_expression(Precedence::Assignment)?;
        while self.consume(Token::In) {
            let start = self.parse_expression(Precedence::Assignment)?;
            value = if self.consume(Token::To) {
                let end = self.parse_expression(Precedence::Assignment)?;
                let step = if self.consume_step() {
                    Some(self.parse_expression(Precedence::Assignment)?)
                } else {
                    None
                };
                self.make_expr(Expression::InRange {
                    value,
                    start,
                    end,
                    step,
                })
            } else {
                self.make_expr(Expression::Binary {
                    op: BinaryOp::In,
                    lhs_expr: value,
                    rhs_expr: start,
                })
            };
        }

        Ok(value)
    }

    fn parse_assignment_expression(&mut self) -> ParseResult<ExpressionId> {
        let lhs_expr = self.parse_expression(Precedence::Ternary)?;
        let Some((token, _)) = self.peek() else {
            return Ok(lhs_expr);
        };

        let Some(kind) = token_to_assignment_kind(&token) else {
            return Ok(lhs_expr);
        };

        self.advance()?;
        let rhs_expr = self.parse_expression(Precedence::Assignment)?;

        Ok(self.make_expr(Expression::Assign {
            kind,
            lhs_expr,
            rhs_expr,
        }))
    }

    fn parse_ternary_expression(&mut self) -> ParseResult<ExpressionId> {
        let condition = self.parse_expression(Precedence::LogicalOr)?;
        if !self.consume(Token::Question) {
            return Ok(condition);
        }

        // `ispath(id) ? locate(id) in composition : fallback`
        self.ternary_true_depth += 1;
        let true_result = self.parse_expression(Precedence::In);
        self.ternary_true_depth -= 1;
        let true_expr = true_result?;
        self.expect_next(Token::Colon)?;
        let false_expr = self.parse_expression(Precedence::Ternary)?;

        Ok(self.make_expr(Expression::Ternary {
            condition,
            true_expr,
            false_expr,
        }))
    }

    /// `ismob(I) ? I:client : fallback`
    fn colon_is_access(&self) -> bool {
        let mut nesting = 0usize;
        let mut pending_questions = self.ternary_true_depth.saturating_sub(1);

        for (token, _) in self.tokens.iter().skip(self.cursor + 1) {
            match token {
                Token::ParenLeft | Token::SquareLeft | Token::BraceLeft | Token::InterpStringBegin(_) => nesting += 1,
                Token::ParenRight | Token::SquareRight | Token::BraceRight | Token::InterpStringEnd(_) => {
                    if nesting == 0 {
                        return false;
                    }
                    nesting -= 1;
                },
                Token::InterpStringMid(_) if nesting == 0 => return false,
                _ if nesting > 0 => {},
                Token::Question => pending_questions += 1,
                Token::Colon if pending_questions == 0 => return true,
                Token::Colon => pending_questions -= 1,
                Token::Comma | Token::Semicolon | Token::Eof => return false,
                _ if token.is_layout() => return false,
                _ => {},
            }
        }

        false
    }

    fn parse_binary_expression(&mut self, precedence: Precedence) -> ParseResult<ExpressionId> {
        let tighter = precedence.next_tighter();
        let mut lhs_expr = self.parse_expression(tighter)?;

        while let Some((token, _)) = self.peek() {
            if token_to_precedence(&token) != precedence {
                break;
            }

            let Some(op) = token_to_binary_op(&token) else {
                break;
            };

            self.advance()?;
            let rhs_precedence = if precedence.is_right_associative() {
                precedence
            } else {
                tighter
            };
            let rhs_expr = self.parse_expression(rhs_precedence)?;
            lhs_expr = self.make_expr(Expression::Binary { op, lhs_expr, rhs_expr });
        }

        Ok(lhs_expr)
    }

    fn parse_unary_expression(&mut self) -> ParseResult<ExpressionId> {
        if let Some((token, _)) = self.peek()
            && let Some(op) = token_to_unary_op(&token)
        {
            self.advance()?;
            let operand = self.parse_expression(Precedence::Unary)?;
            return Ok(self.make_expr(Expression::Unary { op, operand }));
        }

        self.parse_expression(Precedence::Postfix)
    }

    fn parse_postfix_expression(&mut self) -> ParseResult<ExpressionId> {
        let mut expression = self.parse_primary_expression()?;

        loop {
            if self.consume(Token::Increment) {
                expression = self.make_expr(Expression::Unary {
                    op: UnaryOp::PostIncrement,
                    operand: expression,
                });
                continue;
            }

            if self.consume(Token::Decrement) {
                expression = self.make_expr(Expression::Unary {
                    op: UnaryOp::PostDecrement,
                    operand: expression,
                });
                continue;
            }

            if self.peek_is(Token::ParenLeft) {
                let args = self.parse_arguments()?;
                expression = self.make_expr(Expression::Call {
                    callee: expression,
                    args,
                });
                continue;
            }

            if self.peek_is(Token::SquareLeft) || self.peek_is(Token::SafeSquare) {
                let conditional = self.consume(Token::SafeSquare);
                if !conditional {
                    self.expect_next(Token::SquareLeft)?;
                }

                let index = self.parse_isolated_expression(Precedence::Lowest)?;
                self.expect_next(Token::SquareRight)?;
                expression = self.make_expr(Expression::Index {
                    object: expression,
                    index,
                    conditional,
                });
                continue;
            }

            let access = match self.peek().map(|entry| entry.0) {
                _ if self.skip_dangling_access() => break,
                Some(Token::Dot) => AccessKind::Dot,
                Some(Token::Colon) if self.ternary_true_depth == 0 || self.colon_is_access() => AccessKind::Colon,
                Some(Token::SafeDot) => AccessKind::SafeDot,
                Some(Token::SafeColon) => AccessKind::SafeColon,
                Some(Token::Scope) => AccessKind::Scope,
                _ => break,
            };
            self.advance()?;
            let name = self.parse_identifier()?;
            expression = self.make_expr(Expression::Field {
                object: expression,
                name,
                access,
            });
        }

        Ok(expression)
    }

    fn parse_primary_expression(&mut self) -> ParseResult<ExpressionId> {
        let (token, location) = self.advance()?;
        match token {
            Token::Null => Ok(self.make_expr(Expression::Literal(Literal::Null))),
            Token::IntegerLiteral(text) | Token::FloatingPointLiteral(text) => {
                let value = match text.split_once('#') {
                    Some((_, "INF")) => f32::INFINITY,
                    Some(_) => f32::NAN,
                    None => text.parse::<f32>().map_err(|_| ParseError::invalid_token(location))?,
                };
                Ok(self.make_expr(Expression::Literal(Literal::Num(value))))
            },
            Token::HexLiteral(text) => {
                let value = u32::from_str_radix(text.trim_start_matches("0x").trim_start_matches("0X"), 16)
                    .map_err(|_| ParseError::invalid_token(location))?;
                Ok(self.make_expr(Expression::Literal(Literal::Num(value as f32))))
            },
            Token::StringLiteral(text) | Token::RawStringLiteral(text) => {
                Ok(self.make_expr(Expression::Literal(Literal::String(text.to_string()))))
            },
            Token::ResourceLiteral(text) => {
                Ok(self.make_expr(Expression::Literal(Literal::Resource(text.to_string()))))
            },
            Token::InterpStringBegin(begin) => self.parse_interpolated_string(begin),
            Token::ParenLeft => {
                let inner = self.parse_isolated_expression(Precedence::Lowest)?;
                self.expect_next(Token::ParenRight)?;
                Ok(self.make_expr(Expression::Grouped(inner)))
            },
            Token::Slash => {
                self.cursor -= 1;
                self.parse_path_expression()
            },
            Token::New => self.parse_new_expression(),
            Token::Super => Ok(self.make_expr(Expression::Builtin(Builtin::Super))),
            Token::Dot
                if self.peek_is(Token::Soft(SoftKeyword::Proc)) || self.peek_is(Token::Soft(SoftKeyword::Verb)) =>
            {
                self.cursor -= 1;
                self.parse_path_expression()
            },
            Token::Dot => Ok(self.make_expr(Expression::Builtin(Builtin::Dot))),
            Token::Scope => {
                let object = self.make_expr(Expression::Builtin(Builtin::Global));
                let name = self.parse_identifier()?;
                Ok(self.make_expr(Expression::Field {
                    object,
                    name,
                    access: AccessKind::Scope,
                }))
            },
            Token::Soft(keyword) => self.parse_soft_keyword_expression(keyword),
            Token::Identifier(name) => Ok(self.make_expr(Expression::Identifier(Identifier::from(name.to_string())))),
            other => Err(ParseError::unexpected("an expression", other, location)),
        }
    }

    /// `var = 1`
    fn parse_soft_keyword_expression(&mut self, keyword: SoftKeyword) -> ParseResult<ExpressionId> {
        if let Some(builtin) = soft_builtin(keyword) {
            return Ok(self.make_expr(Expression::Builtin(builtin)));
        }

        if self.peek_is(Token::ParenLeft) {
            match keyword {
                SoftKeyword::List => {
                    let args = self.parse_arguments()?;
                    return Ok(self.make_expr(Expression::List(args)));
                },
                SoftKeyword::Pick => return self.parse_pick_expression(),
                SoftKeyword::Input => return self.parse_input_expression(),
                _ => {},
            }
        }

        Ok(self.make_expr(Expression::Identifier(Identifier::from(keyword.as_word().to_string()))))
    }

    fn parse_isolated_expression(&mut self, precedence: Precedence) -> ParseResult<ExpressionId> {
        let depth = self.ternary_true_depth;
        self.ternary_true_depth = 0;
        let result = self.parse_expression(precedence);
        self.ternary_true_depth = depth;

        result
    }

    fn parse_path_expression(&mut self) -> ParseResult<ExpressionId> {
        let path = self.parse_path()?;
        if self.peek_is(Token::BraceLeft) {
            self.parse_modified_type(path)
        } else {
            Ok(self.make_expr(Expression::Path(path)))
        }
    }

    fn parse_modified_type(&mut self, path: TreePath) -> ParseResult<ExpressionId> {
        self.expect_next(Token::BraceLeft)?;
        let mut overrides = Vec::new();
        self.skip_override_delimiters();
        while !self.consume(Token::BraceRight) {
            let name = self.parse_identifier()?;
            self.expect_next(Token::Equal)?;
            let value = self.parse_isolated_expression(Precedence::Lowest)?;
            overrides.push((name, value));

            if !self.peek_is(Token::BraceRight) && !self.skip_override_delimiters() {
                let (token, location) = self.peek().ok_or_else(ParseError::end_of_file)?;
                return Err(ParseError::unexpected(
                    "a modified-type separator or }",
                    token,
                    location,
                ));
            }
        }

        Ok(self.make_expr(Expression::ModifiedType { path, overrides }))
    }

    fn skip_override_delimiters(&mut self) -> bool {
        let start = self.cursor;
        while self.peek().is_some_and(|(token, _)| {
            matches!(
                token,
                Token::Comma | Token::Semicolon | Token::Newline | Token::Indent | Token::Dedent
            )
        }) {
            self.cursor += 1;
        }

        self.cursor != start
    }

    fn parse_new_expression(&mut self) -> ParseResult<ExpressionId> {
        if self.peek_is(Token::ParenLeft) {
            let args = self.parse_arguments()?;
            return Ok(self.make_expr(Expression::New { type_expr: None, args }));
        }

        let mut type_expr = match self.peek().map(|entry| entry.0) {
            Some(Token::Slash) => Some(self.parse_path_expression()?),
            Some(Token::Dot) => {
                self.advance()?;
                Some(self.make_expr(Expression::Builtin(Builtin::Dot)))
            },
            Some(Token::Scope) => Some(self.parse_primary_expression()?),
            Some(token) if token.is_identifier() => {
                self.advance()?;
                Some(self.make_expr(Expression::Identifier(Identifier::from(
                    token.identifier_name().unwrap_or_default().to_string(),
                ))))
            },
            _ => None,
        };

        while let Some(object) = type_expr {
            let access = match self.peek().map(|entry| entry.0) {
                _ if self.skip_dangling_access() => {
                    type_expr = Some(object);
                    break;
                },
                Some(Token::Dot) => AccessKind::Dot,
                Some(Token::Colon) if self.ternary_true_depth == 0 || self.colon_is_access() => AccessKind::Colon,
                Some(Token::SafeDot) => AccessKind::SafeDot,
                Some(Token::SafeColon) => AccessKind::SafeColon,
                Some(Token::Scope) => AccessKind::Scope,
                _ => {
                    type_expr = Some(object);
                    break;
                },
            };
            self.advance()?;
            let name = self.parse_identifier()?;
            type_expr = Some(self.make_expr(Expression::Field { object, name, access }));
        }

        let args = if self.peek_is(Token::ParenLeft) {
            self.parse_arguments()?
        } else {
            Vec::new()
        };

        Ok(self.make_expr(Expression::New { type_expr, args }))
    }

    fn parse_arguments(&mut self) -> ParseResult<Vec<Argument>> {
        self.expect_next(Token::ParenLeft)?;
        let mut args = Vec::new();
        if self.consume(Token::ParenRight) {
            return Ok(args);
        }

        loop {
            if self.consume(Token::Comma) {
                args.push(Argument { key: None, value: None });

                if self.consume(Token::ParenRight) {
                    args.push(Argument { key: None, value: None });
                    break;
                }
                continue;
            }

            if self.consume(Token::ParenRight) {
                args.push(Argument { key: None, value: None });
                break;
            }

            let expression = self.parse_isolated_expression(Precedence::Lowest)?;
            args.push(self.argument_from_expression(expression));

            if self.consume(Token::ParenRight) {
                break;
            }

            self.expect_next(Token::Comma)?;
        }

        Ok(args)
    }

    fn argument_from_expression(&self, expression: ExpressionId) -> Argument {
        match self.expressions.get(expression.index()) {
            Some(Expression::Assign {
                kind: AssignmentKind::Assign,
                lhs_expr,
                rhs_expr,
            }) => Argument {
                key: Some(*lhs_expr),
                value: Some(*rhs_expr),
            },
            _ => Argument {
                key: None,
                value: Some(expression),
            },
        }
    }

    fn parse_pick_expression(&mut self) -> ParseResult<ExpressionId> {
        self.expect_next(Token::ParenLeft)?;
        let mut values = Vec::new();
        while !self.consume(Token::ParenRight) {
            let first = self.parse_isolated_expression(Precedence::Lowest)?;
            let (weight, value) = if self.consume(Token::Semicolon) {
                (Some(first), self.parse_isolated_expression(Precedence::Lowest)?)
            } else {
                (None, first)
            };
            values.push(PickValue { weight, value });

            if self.consume(Token::ParenRight) {
                break;
            }

            self.expect_next(Token::Comma)?;
        }

        Ok(self.make_expr(Expression::Pick(values)))
    }

    fn parse_input_expression(&mut self) -> ParseResult<ExpressionId> {
        let args = self.parse_arguments()?;
        let input_type = if self.consume(Token::As) {
            self.parse_type_spec()?.flags
        } else {
            InputType::NONE
        };

        let in_list = if self.consume(Token::In) {
            Some(self.parse_expression(Precedence::Lowest)?)
        } else {
            None
        };

        Ok(self.make_expr(Expression::Input {
            args,
            input_type,
            in_list,
        }))
    }

    fn parse_interpolated_string(&mut self, begin: &str) -> ParseResult<ExpressionId> {
        let mut chunks = vec![begin.to_string()];
        let mut expressions = Vec::new();
        loop {
            let empty = matches!(
                self.peek(),
                Some((Token::InterpStringMid(_) | Token::InterpStringEnd(_), _))
            );

            expressions.push(match empty {
                true => None,
                false => Some(self.parse_isolated_expression(Precedence::Lowest)?),
            });

            let (token, location) = self.advance()?;
            match token {
                Token::InterpStringMid(chunk) => chunks.push(chunk.to_string()),
                Token::InterpStringEnd(chunk) => {
                    chunks.push(chunk.to_string());
                    break;
                },
                other => {
                    return Err(ParseError::unexpected(
                        "the end of a string interpolation",
                        other,
                        location,
                    ));
                },
            }
        }

        Ok(self.make_expr(Expression::InterpString { chunks, expressions }))
    }
}

pub fn parse(tokens: &[(Token<'_>, Location)]) -> ParseResult<AST> { Parser::new(tokens).parse() }

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> Vec<(Token<'_>, Location)> {
        let (tokens, errors) = lexer::tokenize(source);
        assert!(errors.is_empty(), "lexer errors: {errors:?}");
        tokens.into_iter().filter(|(token, _)| !token.is_comment()).collect()
    }

    fn parse_source(source: &str) -> AST {
        let tokens = tokens(source);
        match parse(&tokens) {
            Ok(ast) => ast,
            Err(error) => panic!("failed to parse test source: {error}"),
        }
    }

    fn parse_expr(source: &str) -> (Vec<Expression>, ExpressionId) {
        let tokens = tokens(source);
        let mut parser = Parser::new(&tokens);
        let expression = match parser.parse_expression(Precedence::Lowest) {
            Ok(expression) => expression,
            Err(error) => panic!("failed to parse test expression: {error}"),
        };

        assert!(parser.peek_is(Token::Eof));

        (parser.expressions, expression)
    }

    fn proc_body(ast: &AST) -> &[Statement] {
        match &ast.declarations[0] {
            Declaration::Proc { body: Some(body), .. } => body,
            _ => panic!("expected a proc with a body"),
        }
    }

    #[test]
    fn keywords_stay_usable_as_path_segments_and_names() {
        let ast = parse_source("/datum/task/throw\n\tvar/matrix/final = null\n\tvar/datum/thing/verb = null\n");
        let Declaration::Type { path, body, .. } = &ast.declarations[0] else {
            panic!("expected a type declaration")
        };

        assert_eq!(path.to_string(), "/datum/task/throw");

        let names: Vec<_> = body
            .iter()
            .map(|declaration| match declaration {
                Declaration::Var { path, .. } => path.name().map(ToString::to_string).unwrap_or_default(),
                other => panic!("expected a var, got {other:?}"),
            })
            .collect();
        assert_eq!(names, vec!["final", "verb"]);

        let ast = parse_source("/proc/f()\n\tvar/matrix/final = null\n\tfinal.Turn(1)\n\tstep(src, 1)\n\tvar x = 1\n");
        assert_eq!(proc_body(&ast).len(), 4);
    }

    #[test]
    fn operator_stays_usable_as_a_variable_and_path_name() {
        let ast = parse_source(
            "/datum/example\n\tvar/operator\n\tvar/mob/operator = null\n\tvar/operator[]\n/datum/operator/child\n",
        );
        let Declaration::Type { body, .. } = &ast.declarations[0] else {
            panic!("expected a type declaration")
        };

        let Declaration::Var { path, var_type, .. } = &body[0] else {
            panic!("expected an untyped variable")
        };
        assert_eq!(path.name().map(Identifier::as_str), Some("operator"));
        assert!(var_type.is_none());
        assert!(!path.flags.contains(PathFlags::IS_OPERATOR));

        let Declaration::Var { path, var_type, .. } = &body[1] else {
            panic!("expected a typed variable")
        };
        assert_eq!(path.name().map(Identifier::as_str), Some("operator"));
        assert_eq!(var_type.as_ref().map(ToString::to_string).as_deref(), Some("/mob"));

        let Declaration::Var { path, dimensions, .. } = &body[2] else {
            panic!("expected an array variable")
        };
        assert_eq!(path.name().map(Identifier::as_str), Some("operator"));
        assert_eq!(dimensions.len(), 1);

        let Declaration::Type { path, .. } = &ast.declarations[1] else {
            panic!("expected a type declaration with an operator segment")
        };
        assert_eq!(path.to_string(), "/datum/operator/child");
    }

    #[test]
    fn soft_keywords_are_read_by_position() {
        let ast = parse_source("/datum/pick/list\n\tvar/step = 1\n\tvar/global/src = 2\n");
        let Declaration::Type { path, body, .. } = &ast.declarations[0] else {
            panic!("expected a type declaration")
        };

        assert_eq!(path.to_string(), "/datum/pick/list");

        let vars: Vec<_> = body
            .iter()
            .map(|declaration| match declaration {
                Declaration::Var { path, .. } => (path.name().map(ToString::to_string).unwrap_or_default(), path.flags),
                other => panic!("expected a var, got {other:?}"),
            })
            .collect();
        assert_eq!(vars[0].0, "step");
        assert_eq!(vars[1].0, "src");
        assert!(vars[1].1.contains(PathFlags::IS_GLOBAL));

        let (expressions, root) = parse_expr("list(src, step(usr, 1), world)");
        let Expression::List(args) = &expressions[root.index()] else {
            panic!("expected a list literal")
        };

        let values: Vec<_> = args
            .iter()
            .map(|argument| argument.value.and_then(|value| expressions.get(value.index())))
            .collect();
        assert!(matches!(values[0], Some(Expression::Builtin(Builtin::Src))));
        assert!(matches!(values[2], Some(Expression::Builtin(Builtin::World))));

        let Some(Expression::Call { callee, .. }) = values[1] else {
            panic!("expected `step` to stay a call")
        };

        assert!(matches!(&expressions[callee.index()], Expression::Identifier(name) if name.as_str() == "step"));
    }

    #[test]
    fn parses_proc_reference_paths() {
        let (expressions, root) = parse_expr("nameof(.proc/Life)");
        let Expression::Call { args, .. } = &expressions[root.index()] else {
            panic!("expected a call")
        };

        let Some(value) = args[0].value else {
            panic!("expected a filled argument")
        };

        let Expression::Path(path) = &expressions[value.index()] else {
            panic!("expected a path argument")
        };

        assert!(!path.absolute);
        assert!(path.flags.contains(PathFlags::IS_PROC));

        let (expressions, root) = parse_expr("nameof(/mob/living.proc/Life)");
        let Expression::Call { args, .. } = &expressions[root.index()] else {
            panic!("expected a call")
        };

        let Some(value) = args[0].value else {
            panic!("expected a filled argument")
        };

        let Expression::Path(path) = &expressions[value.index()] else {
            panic!("expected a path argument")
        };

        assert!(path.absolute);
        assert!(path.flags.contains(PathFlags::IS_PROC));
        assert_eq!(path.declaration_owner(), &path.segments[..2]);
    }

    #[test]
    fn ternary_colons_give_way_to_member_access() {
        let (expressions, root) = parse_expr("ismob(I) ? I:client : fallback");
        let Expression::Ternary { true_expr, .. } = expressions[root.index()] else {
            panic!("expected a ternary")
        };

        assert!(matches!(
            expressions[true_expr.index()],
            Expression::Field {
                access: AccessKind::Colon,
                ..
            }
        ));

        for source in ["a ? b : c", "a?b:c"] {
            let (expressions, root) = parse_expr(source);
            let Expression::Ternary { true_expr, .. } = expressions[root.index()] else {
                panic!("expected a ternary for {source}")
            };

            assert!(matches!(expressions[true_expr.index()], Expression::Identifier(_)));
        }

        let (expressions, root) = parse_expr("a ? b ? null : call(x) : fallback");
        let Expression::Ternary { true_expr, .. } = expressions[root.index()] else {
            panic!("expected a ternary")
        };

        assert!(matches!(expressions[true_expr.index()], Expression::Ternary { .. }));

        let (expressions, root) = parse_expr("a ? locate(b) in c : d");
        let Expression::Ternary { true_expr, .. } = expressions[root.index()] else {
            panic!("expected a ternary")
        };

        assert!(matches!(
            expressions[true_expr.index()],
            Expression::Binary { op: BinaryOp::In, .. }
        ));
    }

    #[test]
    fn parses_declaration_forms_byond_allows() {
        let ast = parse_source("/datum/admin_verb/debug/visibility_flag = 4\n");
        let Declaration::Var { path, initializer, .. } = &ast.declarations[0] else {
            panic!("expected a var declaration")
        };

        assert_eq!(path.declaration_owner().len(), 3);
        assert!(initializer.is_some());

        let ast = parse_source("/datum/thing/New(..., serialized)\n\treturn\n");
        let Declaration::Proc { params, variadic, .. } = &ast.declarations[0] else {
            panic!("expected a proc")
        };

        assert!(variadic);
        assert_eq!(params.len(), 1);

        let ast = parse_source("/datum/timer/proc/operator\"\"()\n\treturn \"now\"\n");
        let Declaration::Proc { path, .. } = &ast.declarations[0] else {
            panic!("expected a proc")
        };

        assert_eq!(path.name().map(ToString::to_string).unwrap_or_default(), "operator\"\"");

        let ast = parse_source("/mob/proc/temperature_expose(null, temp, volume)\n\treturn\n");
        let Declaration::Proc { params, .. } = &ast.declarations[0] else {
            panic!("expected a proc")
        };

        assert_eq!(params.len(), 3);
        assert_eq!(params[0].spec.name.as_str(), "null");
    }

    #[test]
    fn statement_bodies_may_be_empty_and_loops_may_filter_bare_names() {
        let ast = parse_source("/proc/f()\n\ttry\n\t\tg()\n\tcatch\n\treturn 1\n");
        assert_eq!(proc_body(&ast).len(), 2);

        let ast = parse_source("/proc/f()\n\tfor(point as anything in grid)\n\t\tpoint.go()\n");
        let [Statement::For(loop_)] = proc_body(&ast) else {
            panic!("expected a for loop")
        };

        let ForLoop::List { value, .. } = loop_.as_ref() else {
            panic!("expected a list loop")
        };

        assert!(!value.declares);
        assert!(value.spec.as_type.is_some());
    }

    #[test]
    fn parses_braced_declaration_blocks() {
        let ast =
            parse_source("/datum/bitfield/check_flags { flags = list(\"A\" = 1); variable = \"check_flags\"; }\n");
        let Declaration::Type { path, body, .. } = &ast.declarations[0] else {
            panic!("expected a type declaration")
        };

        assert_eq!(path.to_string(), "/datum/bitfield/check_flags");
        assert_eq!(body.len(), 2);
        assert!(body.iter().all(|d| matches!(d, Declaration::Override { .. })));

        let ast = parse_source("/obj/thing\n{\n\tname = \"thing\"\n\tvar/count = 3\n}\n");
        let Declaration::Type { body, .. } = &ast.declarations[0] else {
            panic!("expected a type declaration")
        };

        assert!(matches!(body[0], Declaration::Override { .. }));
        assert!(matches!(body[1], Declaration::Var { .. }));

        let ast = parse_source("var { a = 1; b = 2 }\n");
        assert_eq!(ast.declarations.len(), 2);
        assert!(ast.declarations.iter().all(|d| matches!(d, Declaration::Var { .. })));
    }

    #[test]
    fn expression_precedence_and_associativity_match_dm() {
        let (expressions, root) = parse_expr("1 + 2 * 3 < 8 << 1 == 0");
        let Expression::Binary {
            op: BinaryOp::CompEq,
            lhs_expr,
            ..
        } = expressions[root.index()]
        else {
            panic!("expected equality at the root")
        };

        let Expression::Binary {
            op: BinaryOp::ShiftLeft,
            lhs_expr: comparison,
            ..
        } = expressions[lhs_expr.index()]
        else {
            panic!("expected shift below equality")
        };

        assert!(matches!(
            expressions[comparison.index()],
            Expression::Binary {
                op: BinaryOp::CompLess,
                ..
            }
        ));

        let (expressions, root) = parse_expr("a = b = 1");
        let Expression::Assign { rhs_expr, .. } = expressions[root.index()] else {
            panic!("expected assignment")
        };

        assert!(matches!(expressions[rhs_expr.index()], Expression::Assign { .. }));

        let (expressions, root) = parse_expr("2 ** 3 ** 2");
        let Expression::Binary {
            op: BinaryOp::Pow,
            lhs_expr,
            ..
        } = expressions[root.index()]
        else {
            panic!("expected power")
        };

        assert!(matches!(
            expressions[lhs_expr.index()],
            Expression::Binary { op: BinaryOp::Pow, .. }
        ));
    }

    #[test]
    fn parses_nested_ternaries_and_in_ranges() {
        let (expressions, root) = parse_expr("flag ? one : other ? two : three");
        let Expression::Ternary { false_expr, .. } = expressions[root.index()] else {
            panic!("expected ternary")
        };

        assert!(matches!(expressions[false_expr.index()], Expression::Ternary { .. }));

        let (expressions, root) = parse_expr("value in 1 to 10 step 2");
        assert!(matches!(
            expressions[root.index()],
            Expression::InRange { step: Some(_), .. }
        ));
    }

    #[test]
    fn parses_special_expression_forms() {
        let (expressions, root) = parse_expr("list(1,, \"a\" = 2, (b = 3))");
        let Expression::List(args) = &expressions[root.index()] else {
            panic!("expected list")
        };

        assert_eq!(args.len(), 4);
        assert!(args[1].value.is_none());
        assert!(args[2].key.is_some());
        let grouped = args[3].value.expect("grouped assignment argument");
        assert!(matches!(expressions[grouped.index()], Expression::Grouped(_)));

        let (expressions, root) = parse_expr("pick(10; \"rare\", \"common\")");
        assert!(matches!(
            &expressions[root.index()],
            Expression::Pick(values) if values.len() == 2 && values[0].weight.is_some()
        ));

        let (expressions, root) = parse_expr("input(src, \"pick\") as text|null in list(\"a\")");
        assert!(matches!(
            expressions[root.index()],
            Expression::Input {
                input_type,
                in_list: Some(_),
                ..
            } if input_type.contains(InputType::TEXT) && input_type.contains(InputType::NULL)
        ));

        let (expressions, root) = parse_expr("new /obj/item { name = \"custom\"; value = 2 }(src)");
        let Expression::New {
            type_expr: Some(type_expr),
            args,
        } = &expressions[root.index()]
        else {
            panic!("expected typed new")
        };

        assert_eq!(args.len(), 1);
        assert!(matches!(
            &expressions[type_expr.index()],
            Expression::ModifiedType { overrides, .. } if overrides.len() == 2
        ));

        let (expressions, root) = parse_expr("\"hello [src], [name]\"");
        assert!(matches!(
            &expressions[root.index()],
            Expression::InterpString { chunks, expressions } if chunks.len() == 3 && expressions.len() == 2
        ));
    }

    /// `[/* comment */]` embeds nothing, and SS13 browser code writes it inside `{"..."}` blocks
    #[test]
    fn an_empty_interpolation_keeps_its_slot() {
        let (expressions, root) = parse_expr("\"a[]b[src]c\"");
        let Expression::InterpString {
            chunks,
            expressions: parts,
        } = &expressions[root.index()]
        else {
            panic!("expected an interpolated string")
        };

        assert_eq!(chunks, &["a", "b", "c"]);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].is_none());
        assert!(parts[1].is_some());
    }

    /// `CLERIC_T3 = /datum/action/bloodrage.` is a typo BYOND compiles, so it cannot be fatal
    #[test]
    fn dangling_access_and_path_separators_are_not_fatal() {
        let (expressions, root) = parse_expr("list(a = /datum/foo.)");
        assert!(matches!(&expressions[root.index()], Expression::List(args) if args.len() == 1));

        let (expressions, root) = parse_expr("src.");
        assert!(matches!(&expressions[root.index()], Expression::Builtin(Builtin::Src)));

        let (expressions, root) = parse_expr("src.name");
        assert!(matches!(&expressions[root.index()], Expression::Field { .. }));

        let (expressions, root) = parse_expr("/obj/effect/decal/cleanable/ in M");
        let Expression::Binary {
            op: BinaryOp::In,
            lhs_expr,
            ..
        } = &expressions[root.index()]
        else {
            panic!("expected a membership expression")
        };
        assert!(matches!(&expressions[lhs_expr.index()], Expression::Path(path)
            if path.to_string() == "/obj/effect/decal/cleanable"));

        let (expressions, root) = parse_expr("/datum/in");
        assert!(matches!(&expressions[root.index()], Expression::Path(path) if path.to_string() == "/datum/in"));
    }

    #[test]
    fn parses_statement_families_and_typed_bindings() {
        let ast = parse_source(
            r#"
/proc/test(mob/user, amount = 1 as num in 1 to 10, ...)
    var/list/items[5], other = list()
    set waitfor = 0
    if(!user)
        return
    else if(amount > 2)
        world << "hi"
    else
        user >> amount
    while(amount)
        amount--
    do
        amount++
    while(amount < 2)
    spawn(1)
        del user
    try
        throw user
    catch(/datum/error/e)
        goto done
    obj:field()
    done:
        break
"#,
        );

        let Declaration::Proc {
            params,
            variadic,
            body: Some(body),
            ..
        } = &ast.declarations[0]
        else {
            panic!("expected proc")
        };

        assert!(*variadic);
        assert_eq!(params[0].spec.name.as_str(), "user");
        assert_eq!(params[0].spec.var_type.as_ref().unwrap().to_string(), "/mob");
        assert!(matches!(body[0], Statement::Var { .. }));
        assert!(matches!(body[2], Statement::Setting { .. }));
        assert!(matches!(body[3], Statement::If { .. }));
        assert!(
            body.iter()
                .any(|statement| matches!(statement, Statement::TryCatch { .. }))
        );
        assert!(
            body.iter()
                .any(|statement| matches!(statement, Statement::Expression(_)))
        );
        assert!(
            body.iter()
                .any(|statement| matches!(statement, Statement::Label { .. }))
        );
    }

    #[test]
    fn parses_all_for_header_forms() {
        let ast = parse_source(
            r#"
/proc/test()
    for(;;) break
    for(var/i; i < 10; i++) continue
    for(var/two, two < 2) continue
    for(i = 1 to 10 step 2) continue
    for(var/j in 1 to 5) continue
    for(var/key, value in list()) continue
    for(existing in world) continue
"#,
        );
        let body = proc_body(&ast);
        assert_eq!(body.len(), 7);
        assert!(matches!(&body[0], Statement::For(loop_) if matches!(**loop_, ForLoop::Standard { init: None, .. })));
        assert!(
            matches!(&body[1], Statement::For(loop_) if matches!(**loop_, ForLoop::Standard { init: Some(_), .. }))
        );
        assert!(matches!(&body[2], Statement::For(loop_) if matches!(**loop_, ForLoop::Standard { step: None, .. })));
        assert!(
            matches!(&body[3], Statement::For(loop_) if matches!(**loop_, ForLoop::Range { variable: LoopBinding { declares: false, .. }, .. }))
        );
        assert!(
            matches!(&body[4], Statement::For(loop_) if matches!(**loop_, ForLoop::Range { variable: LoopBinding { declares: true, .. }, .. }))
        );
        assert!(matches!(&body[5], Statement::For(loop_) if matches!(**loop_, ForLoop::List { key: Some(_), .. })));
        assert!(matches!(&body[6], Statement::For(loop_) if matches!(**loop_, ForLoop::List { key: None, .. })));
    }

    #[test]
    fn parses_switch_and_braced_bodies() {
        let ast = parse_source(
            r#"
/proc/indented(value)
    switch(value)
        if(1, 2 to 4)
            return 1
        else
            return 0
/proc/braced() { if(1) { return 1; }; else { return 2; }; }
"#,
        );
        let Declaration::Proc { body: Some(body), .. } = &ast.declarations[0] else {
            panic!("expected proc")
        };

        assert!(matches!(
            &body[0],
            Statement::Switch { cases, default: Some(_), .. }
                if cases.len() == 1 && cases[0].values.len() == 2
        ));
        assert!(matches!(
            &ast.declarations[1],
            Declaration::Proc { body: Some(body), .. } if matches!(body[0], Statement::If { else_branch: Some(_), .. })
        ));
    }

    #[test]
    fn indented_blocks_chain_var_and_proc_prefixes() {
        let ast = parse_source(
            "var/const\n\tNORTH = 1\n\tSOUTH = 2\nvar\n\tmob/owner = null\n\tconst\n\t\tVERSION = \"1.0\"\n",
        );
        let names: Vec<_> = ast
            .declarations
            .iter()
            .map(|declaration| match declaration {
                Declaration::Var {
                    path,
                    var_type,
                    modifiers,
                    initializer,
                    ..
                } => (
                    path.name().unwrap().as_str().to_string(),
                    var_type.as_ref().map(TreePath::to_string),
                    modifiers.is_const,
                    initializer.is_some(),
                ),
                other => panic!("expected a var, got {other:?}"),
            })
            .collect();
        assert_eq!(
            names,
            vec![
                ("NORTH".into(), None, true, true),
                ("SOUTH".into(), None, true, true),
                ("owner".into(), Some("/mob".into()), false, true),
                ("VERSION".into(), None, true, true),
            ]
        );

        let ast = parse_source("mob\n\tproc\n\t\tgreet()\n\t\t\treturn 1\n\tverb\n\t\tpoke()\n\t\t\treturn\n");
        let Declaration::Type { body, .. } = &ast.declarations[0] else {
            panic!("expected a type")
        };

        assert!(matches!(&body[0], Declaration::Proc { kind: ProcKind::Proc, path, .. }
            if path.name().unwrap().as_str() == "greet" && path.declaration_owner().is_empty()));
        assert!(matches!(
            &body[1],
            Declaration::Proc {
                kind: ProcKind::Verb,
                ..
            }
        ));
    }

    #[test]
    fn indented_var_blocks_chain_inside_proc_bodies() {
        let ast = parse_source("/proc/test()\n\tvar\n\t\ta = 1\n\t\tmob/M = null\n\treturn a\n");
        let body = proc_body(&ast);
        assert!(matches!(&body[0], Statement::Var { spec, initializer: Some(_), .. }
            if spec.name.as_str() == "a" && spec.var_type.is_none()));
        assert!(matches!(&body[1], Statement::Var { spec, .. }
            if spec.name.as_str() == "M" && spec.var_type.as_ref().unwrap().to_string() == "/mob"));
        assert!(matches!(body[2], Statement::Return(Some(_))));
    }

    #[test]
    fn accepts_slice_exhaustion_and_rejects_missing_statement_delimiters() {
        let mut no_eof = tokens("/obj/item");
        assert_eq!(no_eof.pop().unwrap().0, Token::Eof);
        assert!(parse(&no_eof).is_ok());

        let bad = tokens("/proc/test()\n    return 1 unexpected");
        assert!(parse(&bad).is_err());
    }

    #[test]
    fn parses_every_overloadable_operator_name() {
        let names = [
            "operator+",
            "operator-",
            "operator*",
            "operator/",
            "operator%",
            "operator%%",
            "operator**",
            "operator+=",
            "operator-=",
            "operator*=",
            "operator/=",
            "operator%=",
            "operator%%=",
            "operator**=",
            "operator&",
            "operator|",
            "operator^",
            "operator~",
            "operator&=",
            "operator|=",
            "operator^=",
            "operator<<",
            "operator>>",
            "operator<<=",
            "operator>>=",
            "operator==",
            "operator!=",
            "operator<>",
            "operator<=>",
            "operator~=",
            "operator~!",
            "operator<",
            "operator>",
            "operator<=",
            "operator>=",
            "operator++",
            "operator--",
            "operator[]",
            "operator[]=",
            "operator()",
            "operator:=",
        ];

        for name in names {
            let ast = parse_source(&format!("/datum/proc/{name}(a)\n\treturn a\n"));
            assert!(
                matches!(&ast.declarations[0], Declaration::Proc { kind: ProcKind::Operator, path, .. }
                    if path.name().unwrap().as_str() == name && path.flags.contains(PathFlags::IS_OPERATOR)),
                "{name} did not parse as an operator"
            );
        }
    }

    #[test]
    fn rejects_non_overloadable_operator_names() {
        assert!(parse(&tokens("/datum/proc/operator&&(a)\n\treturn a\n")).is_err());
        assert!(parse(&tokens("/datum/proc/operator!(a)\n\treturn a\n")).is_err());
    }

    #[test]
    fn proc_bodies_may_sit_on_the_header_line() {
        let ast = parse_source("/datum\n\tproc\n\t\toperator:=(a) src.x = a\n\t\tMultiply(m) m = 1; return m\n");
        let Declaration::Type { body, .. } = &ast.declarations[0] else {
            panic!("expected a type declaration")
        };

        assert!(
            matches!(&body[0], Declaration::Proc { kind: ProcKind::Operator, body: Some(body), .. }
            if body.len() == 1)
        );
        assert!(matches!(&body[1], Declaration::Proc { body: Some(body), .. } if body.len() == 2));
    }

    #[test]
    fn comma_separated_vars_share_their_declared_type() {
        let ast = parse_source("/mob\n\tvar/mob/M = null, N = null\n\tvar/const/A = 1, B = 2\n");
        let Declaration::Type { body, .. } = &ast.declarations[0] else {
            panic!("expected a type declaration")
        };

        let typed: Vec<_> = body
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Var {
                    path,
                    var_type,
                    modifiers,
                    ..
                } => Some((
                    path.name()?.as_str(),
                    var_type.as_ref().map(TreePath::to_string),
                    *modifiers,
                )),
                _ => None,
            })
            .collect();

        assert_eq!(typed.len(), 4);
        assert_eq!(typed[0].0, "M");
        assert_eq!(typed[1].0, "N");
        assert_eq!(typed[0].1, typed[1].1);
        assert_eq!(typed[0].1.as_deref(), Some("/mob"));
        assert_eq!(typed[2].0, "A");
        assert_eq!(typed[3].0, "B");
        assert!(typed[2].2.is_const && typed[3].2.is_const);
    }

    #[test]
    fn comma_separated_vars_keep_per_entry_suffixes_and_allow_a_trailing_comma() {
        let ast = parse_source("/mob\n\tvar/list/L[] = list(), K[4]\n\tvar/tmp/x = 1,\n");
        let Declaration::Type { body, .. } = &ast.declarations[0] else {
            panic!("expected a type declaration")
        };

        assert_eq!(body.len(), 3);
        assert!(
            matches!(&body[0], Declaration::Var { path, dimensions, initializer: Some(_), .. }
            if path.name().unwrap().as_str() == "L" && dimensions.len() == 1 && dimensions[0].is_none())
        );
        assert!(
            matches!(&body[1], Declaration::Var { path, dimensions, initializer: None, .. }
            if path.name().unwrap().as_str() == "K" && dimensions.len() == 1 && dimensions[0].is_some())
        );
        assert!(matches!(&body[2], Declaration::Var { path, .. }
            if path.name().unwrap().as_str() == "x"));
    }
}
