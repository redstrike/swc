use self::ctx::Ctx;
use crate::{marks::Marks, option::CompressOptions, util::MoudleItemExt, MAX_PAR_DEPTH};
use rayon::prelude::*;
use swc_common::{pass::Repeated, DUMMY_SP};
use swc_ecma_ast::*;
use swc_ecma_visit::{noop_visit_mut_type, VisitMut, VisitMutWith, VisitWith};

mod arrows;
mod bools;
mod conds;
mod ctx;
mod dead_code;
mod evaluate;
mod loops;
mod misc;
mod numbers;
mod properties;
mod sequences;
mod strings;
mod unsafes;
mod vars;

pub(super) fn pure_optimizer<'a>(
    options: &'a CompressOptions,
    marks: Marks,
) -> impl 'a + VisitMut + Repeated {
    Pure {
        options,
        marks,
        ctx: Default::default(),
        changed: Default::default(),
    }
}

struct Pure<'a> {
    options: &'a CompressOptions,
    marks: Marks,
    ctx: Ctx,
    changed: bool,
}

impl Repeated for Pure<'_> {
    fn changed(&self) -> bool {
        self.changed
    }

    fn reset(&mut self) {
        self.ctx = Default::default();
        self.changed = false;
    }
}

impl Pure<'_> {
    fn handle_stmt_likes<T>(&mut self, stmts: &mut Vec<T>)
    where
        T: MoudleItemExt,
        Vec<T>: VisitWith<self::vars::VarWithOutInitCounter>
            + VisitMutWith<self::vars::VarPrepender>
            + VisitMutWith<self::vars::VarMover>,
    {
        self.remove_dead_branch(stmts);

        self.remove_unreachable_stmts(stmts);

        self.drop_useless_blocks(stmts);

        self.collapse_vars_without_init(stmts);
    }

    /// Visit `nodes`, maybe in parallel.
    fn visit_par<N>(&mut self, nodes: &mut Vec<N>)
    where
        N: for<'aa> VisitMutWith<Pure<'aa>> + Send + Sync,
    {
        if self.ctx.par_depth >= MAX_PAR_DEPTH * 2 || cfg!(target_arch = "wasm32") {
            for node in nodes {
                let mut v = Pure {
                    options: self.options,
                    marks: self.marks,
                    ctx: self.ctx,
                    changed: false,
                };
                node.visit_mut_with(&mut v);

                self.changed |= v.changed;
            }
        } else {
            let results = nodes
                .par_iter_mut()
                .map(|node| {
                    let mut v = Pure {
                        options: self.options,
                        marks: self.marks,
                        ctx: Ctx {
                            par_depth: self.ctx.par_depth + 1,
                            ..self.ctx
                        },
                        changed: false,
                    };
                    node.visit_mut_with(&mut v);

                    v.changed
                })
                .collect::<Vec<_>>();

            for res in results {
                self.changed |= res;
            }
        }
    }
}

impl VisitMut for Pure<'_> {
    noop_visit_mut_type!();

    fn visit_mut_assign_expr(&mut self, e: &mut AssignExpr) {
        {
            let ctx = Ctx {
                is_lhs_of_assign: true,
                ..self.ctx
            };
            e.left.visit_mut_children_with(&mut *self.with_ctx(ctx));
        }

        e.right.visit_mut_with(self);
    }

    fn visit_mut_bin_expr(&mut self, e: &mut BinExpr) {
        e.visit_mut_children_with(self);

        self.compress_cmp_of_typeof_with_lit(e);

        self.optimize_cmp_with_null_or_undefined(e);

        if e.op == op!(bin, "+") {
            self.concat_tpl(&mut e.left, &mut e.right);
        }
    }

    fn visit_mut_block_stmt_or_expr(&mut self, body: &mut BlockStmtOrExpr) {
        body.visit_mut_children_with(self);

        self.optimize_arrow_body(body);
    }

    fn visit_mut_call_expr(&mut self, e: &mut CallExpr) {
        {
            let ctx = Ctx {
                is_callee: true,
                ..self.ctx
            };
            e.callee.visit_mut_with(&mut *self.with_ctx(ctx));
        }

        e.args.visit_mut_with(self);

        self.drop_arguemtns_of_symbol_call(e);
    }

    fn visit_mut_cond_expr(&mut self, e: &mut CondExpr) {
        e.visit_mut_children_with(self);

        self.optimize_expr_in_bool_ctx(&mut e.test);

        self.negate_cond_expr(e);
    }

    fn visit_mut_expr(&mut self, e: &mut Expr) {
        {
            let ctx = Ctx {
                in_first_expr: false,
                ..self.ctx
            };
            e.visit_mut_children_with(&mut *self.with_ctx(ctx));
        }

        match e {
            Expr::Seq(seq) => {
                if seq.exprs.is_empty() {
                    *e = Expr::Invalid(Invalid { span: DUMMY_SP });
                    return;
                }
            }
            _ => {}
        }

        self.eval_opt_chain(e);

        self.eval_number_call(e);

        self.eval_number_method_call(e);

        self.swap_bin_operands(e);

        self.handle_property_access(e);

        self.optimize_bools(e);

        self.drop_logical_operands(e);

        self.lift_minus(e);

        self.convert_tpl_to_str(e);

        self.drop_useless_addition_of_str(e);

        self.compress_useless_deletes(e);

        self.remove_useless_logical_rhs(e);

        self.handle_negated_seq(e);

        self.concat_str(e);

        self.eval_array_method_call(e);

        self.eval_fn_method_call(e);

        self.eval_str_method_call(e);

        self.compress_conds_as_logical(e);

        self.compress_cond_with_logical_as_logical(e);

        self.lift_seqs_of_bin(e);

        self.lift_seqs_of_cond_assign(e);
    }

    fn visit_mut_exprs(&mut self, exprs: &mut Vec<Box<Expr>>) {
        self.visit_par(exprs);
    }

    fn visit_mut_for_stmt(&mut self, s: &mut ForStmt) {
        s.visit_mut_children_with(self);

        self.merge_for_if_break(s);

        if let Some(test) = &mut s.test {
            self.optimize_expr_in_bool_ctx(&mut **test);
        }
    }

    fn visit_mut_function(&mut self, f: &mut Function) {
        {
            let ctx = Ctx {
                in_try_block: false,
                ..self.ctx
            };
            f.visit_mut_children_with(&mut *self.with_ctx(ctx));
        }

        if let Some(body) = &mut f.body {
            self.remove_useless_return(&mut body.stmts);

            if let Some(last) = body.stmts.last_mut() {
                self.drop_unused_stmt_at_end_of_fn(last);
            }
        }
    }

    fn visit_mut_if_stmt(&mut self, s: &mut IfStmt) {
        s.visit_mut_children_with(self);

        self.optimize_expr_in_bool_ctx(&mut s.test);
    }

    fn visit_mut_member_expr(&mut self, e: &mut MemberExpr) {
        e.obj.visit_mut_with(self);
        if e.computed {
            e.prop.visit_mut_with(self);
        }

        self.optimize_property_of_member_expr(e);

        self.handle_known_computed_member_expr(e);
    }

    fn visit_mut_module_items(&mut self, items: &mut Vec<ModuleItem>) {
        self.visit_par(items);

        self.handle_stmt_likes(items);
    }

    fn visit_mut_new_expr(&mut self, e: &mut NewExpr) {
        {
            let ctx = Ctx {
                is_callee: true,
                ..self.ctx
            };
            e.callee.visit_mut_with(&mut *self.with_ctx(ctx));
        }

        e.args.visit_mut_with(self);
    }

    fn visit_mut_prop(&mut self, p: &mut Prop) {
        p.visit_mut_children_with(self);

        self.optimize_arrow_method_prop(p);
    }

    fn visit_mut_prop_name(&mut self, p: &mut PropName) {
        p.visit_mut_children_with(self);

        self.optimize_computed_prop_name_as_normal(p);
        self.optimize_prop_name(p);
    }

    fn visit_mut_prop_or_spreads(&mut self, exprs: &mut Vec<PropOrSpread>) {
        self.visit_par(exprs);
    }

    fn visit_mut_return_stmt(&mut self, s: &mut ReturnStmt) {
        s.visit_mut_children_with(self);

        self.drop_undefined_from_return_arg(s);
    }

    fn visit_mut_seq_expr(&mut self, e: &mut SeqExpr) {
        e.visit_mut_children_with(self);

        self.drop_useless_ident_ref_in_seq(e);

        self.merge_seq_call(e);
    }

    fn visit_mut_stmt(&mut self, s: &mut Stmt) {
        {
            let ctx = Ctx {
                is_update_arg: false,
                is_callee: false,
                in_delete: false,
                in_first_expr: true,
                ..self.ctx
            };
            s.visit_mut_children_with(&mut *self.with_ctx(ctx));
        }

        if self.options.drop_debugger {
            match s {
                Stmt::Debugger(..) => {
                    self.changed = true;
                    *s = Stmt::Empty(EmptyStmt { span: DUMMY_SP });
                    log::debug!("drop_debugger: Dropped a debugger statement");
                    return;
                }
                _ => {}
            }
        }

        self.loop_to_for_stmt(s);
    }

    fn visit_mut_stmts(&mut self, items: &mut Vec<Stmt>) {
        self.visit_par(items);

        self.handle_stmt_likes(items);

        items.retain(|s| match s {
            Stmt::Empty(..) => false,
            _ => true,
        });
    }

    /// We don't optimize [Tpl] contained in [TaggedTpl].
    fn visit_mut_tagged_tpl(&mut self, n: &mut TaggedTpl) {
        n.tag.visit_mut_with(self);
    }

    fn visit_mut_tpl(&mut self, n: &mut Tpl) {
        n.visit_mut_children_with(self);
        debug_assert_eq!(n.exprs.len() + 1, n.quasis.len());

        self.compress_tpl(n);

        debug_assert_eq!(
            n.exprs.len() + 1,
            n.quasis.len(),
            "tagged template literal compressor created an invalid template literal"
        );
    }

    fn visit_mut_try_stmt(&mut self, n: &mut TryStmt) {
        let ctx = Ctx {
            in_try_block: true,
            ..self.ctx
        };
        n.block.visit_mut_with(&mut *self.with_ctx(ctx));

        n.handler.visit_mut_with(self);

        n.finalizer.visit_mut_with(self);
    }

    fn visit_mut_unary_expr(&mut self, e: &mut UnaryExpr) {
        {
            let ctx = Ctx {
                in_delete: e.op == op!("delete"),
                ..self.ctx
            };
            e.visit_mut_children_with(&mut *self.with_ctx(ctx));
        }

        match e.op {
            op!("!") => {
                self.optimize_expr_in_bool_ctx(&mut e.arg);
            }

            op!(unary, "+") | op!(unary, "-") => {
                self.optimize_expr_in_num_ctx(&mut e.arg);
            }
            _ => {}
        }
    }

    fn visit_mut_update_expr(&mut self, e: &mut UpdateExpr) {
        let ctx = Ctx {
            is_update_arg: true,
            ..self.ctx
        };

        e.visit_mut_children_with(&mut *self.with_ctx(ctx));
    }

    fn visit_mut_while_stmt(&mut self, s: &mut WhileStmt) {
        s.visit_mut_children_with(self);

        self.optimize_expr_in_bool_ctx(&mut s.test);
    }
}
