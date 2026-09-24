use compact_str::CompactString;
use core_utils::xxh3;
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_span::SPAN;
use oxc_span::SourceType;
use oxc_traverse::{Traverse, TraverseCtx, traverse_mut};
use smallvec::SmallVec;
use std::sync::Arc;

thread_local! {
    static OXC_ALLOC: std::cell::RefCell<Allocator> = std::cell::RefCell::new(Allocator::new());
}

#[derive(Debug, Clone)]
pub enum Lit {
    Num(f64),
    Str(CompactString),
}

pub struct Normalized {
    pub src: Arc<str>,
    pub skel: u64,
    pub args: SmallVec<[Lit; 16]>,
}

#[derive(Debug, Clone, Copy)]
pub struct NormFail;

const ARGS_CAP: usize = 64;

struct FoldPass;

impl<'a> Traverse<'a, ()> for FoldPass {
    fn exit_expression(&mut self, node: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let (lv, rv, op) = match node {
            Expression::BinaryExpression(be) => match (&be.left, &be.right) {
                (Expression::NumericLiteral(a), Expression::NumericLiteral(b)) => {
                    (a.value, b.value, be.operator)
                }
                _ => return,
            },
            _ => return,
        };
        let folded = match op {
            BinaryOperator::Addition => lv + rv,
            BinaryOperator::Subtraction => lv - rv,
            BinaryOperator::Multiplication => lv * rv,
            BinaryOperator::Division => {
                if rv == 0.0 {
                    return;
                }
                lv / rv
            }
            _ => return,
        };
        if folded.is_finite() {
            *node = Expression::new_numeric_literal(SPAN, folded, None, NumberBase::Decimal, ctx);
        }
    }
}
struct NormCtx<'s> {
    canon: &'s [u32],
    args: &'s mut SmallVec<[Lit; 16]>,
}

struct NormPass;

impl<'a, 's> Traverse<'a, &'s mut NormCtx<'s>> for NormPass {
    fn enter_identifier_reference(
        &mut self,
        node: &mut IdentifierReference<'a>,
        ctx: &mut TraverseCtx<'a, &'s mut NormCtx<'s>>,
    ) {
        let Some(rid) = node.reference_id.get() else {
            return;
        };
        let Some(sid) = ctx.scoping().get_reference(rid).symbol_id() else {
            match node.name.as_str() {
                "__silo_sha256" => node.name = Ident::from_str_in("__$h", ctx),
                "__silo_md5" => node.name = Ident::from_str_in("__$m", ctx),
                _ => {}
            }
            return;
        };
        let idx = ctx.state.canon[sid.index()];
        node.name = canon_ident(idx, ctx);
    }

    fn enter_binding_identifier(
        &mut self,
        node: &mut BindingIdentifier<'a>,
        ctx: &mut TraverseCtx<'a, &'s mut NormCtx<'s>>,
    ) {
        let Some(sid) = node.symbol_id.get() else {
            return;
        };
        let idx = ctx.state.canon[sid.index()];
        node.name = canon_ident(idx, ctx);
    }

    fn exit_expression(
        &mut self,
        node: &mut Expression<'a>,
        ctx: &mut TraverseCtx<'a, &'s mut NormCtx<'s>>,
    ) {
        let lit = match node {
            Expression::StringLiteral(lit) if !lit.lone_surrogates => {
                Lit::Str(CompactString::new(lit.value.as_str()))
            }
            Expression::NumericLiteral(lit) if lit.value.is_finite() => Lit::Num(lit.value),
            _ => return,
        };
        let st = &mut *ctx.state;
        if st.args.len() >= ARGS_CAP {
            return;
        }
        let idx = st.args.len();
        st.args.push(lit);
        let object = Expression::new_identifier(SPAN, "__$a", ctx);
        let index =
            Expression::new_numeric_literal(SPAN, idx as f64, None, NumberBase::Decimal, ctx);
        *node = Expression::new_computed_member_expression(SPAN, object, index, false, ctx);
    }
}

fn canon_ident<'a, A: oxc_allocator::GetAllocator<'a>>(idx: u32, alloc: &A) -> Ident<'a> {
    let mut s = CompactString::with_capacity(8);
    s.push('v');
    core_utils::push_int_into(&mut s, idx as i64);
    Ident::from_str_in(s.as_str(), alloc)
}

pub fn normalize(script: &[u8]) -> Result<Normalized, NormFail> {
    let text = core_utils::utf8::basic::from_utf8(script).map_err(|_| NormFail)?;
    OXC_ALLOC.with(|cell| {
        let mut cell = cell.borrow_mut();
        let alloc: &mut Allocator = &mut cell;
        alloc.reset();
        let ret = Parser::new(alloc, text, SourceType::script()).parse();
        if ret.panicked || !ret.diagnostics.is_empty() {
            return Err(NormFail);
        }
        let mut program = ret.program;
        let sem = SemanticBuilder::new().build(&program);
        if !sem.diagnostics.is_empty() {
            return Err(NormFail);
        }
        let scoping = sem.semantic.into_scoping();
        let mut canon: SmallVec<[u32; 64]> = SmallVec::new();
        for i in 0..scoping.symbols_len() as u32 {
            canon.push(i);
        }
        let scoping = traverse_mut(&mut FoldPass, alloc, &mut program, scoping, ());
        let mut args: SmallVec<[Lit; 16]> = SmallVec::new();
        let mut state = NormCtx {
            canon: &canon,
            args: &mut args,
        };
        let _scoping = traverse_mut(&mut NormPass, alloc, &mut program, scoping, &mut state);
        let ast = oxc_ast::builder::AstBuilder::new(alloc);
        let n = program.body.len();
        if n > 0 {
            let last_slot = &mut program.body[n - 1];
            let taken =
                std::mem::replace(last_slot, Statement::new_return_statement(SPAN, None, &ast));
            *last_slot = match taken {
                Statement::ExpressionStatement(es) => {
                    let un = es.unbox();
                    Statement::new_return_statement(SPAN, Some(un.expression), &ast)
                }
                other => other,
            };
        }
        let code = Codegen::new()
            .with_options(CodegenOptions {
                minify: true,
                ..CodegenOptions::default()
            })
            .build(&program)
            .code;
        let mut wrapped = String::with_capacity(code.len() + 48);
        wrapped.push_str("(function(__$a, __$h, __$m){");
        wrapped.push_str(&code);
        wrapped.push_str("})");
        let skel = xxh3::hash(wrapped.as_bytes());
        Ok(Normalized {
            src: Arc::from(wrapped),
            skel,
            args,
        })
    })
}
