use std::collections::BTreeSet;

use oxc_ast::ast::ArrowFunctionExpression;
use oxc_ast::ast::{
    AssignmentTargetPropertyIdentifier, BindingIdentifier, BindingPattern, BlockStatement, Class,
    Declaration, Expression, ForStatement, ForStatementInit, ForStatementLeft, FormalParameters,
    Function, SimpleAssignmentTarget, Statement, VariableDeclaration, VariableDeclarationKind,
};
use oxc_ast_visit::Visit;
use oxc_span::GetSpan;
use oxc_syntax::scope::ScopeFlags;

use super::analysis::{Analysis, LoopInfo};
use super::edits::Edits;
use super::lines::LineIndex;

const CATCH: &str = "} catch (__xray_error) { __xray.raise(__xray_error); throw __xray_error } finally { __xray.exit() }";

pub struct Instrumenter<'s> {
    source: &'s str,
    lines: &'s LineIndex,
    edits: &'s mut Edits,
    scopes: Vec<Vec<String>>,
    pub analysis: Analysis,
}

#[derive(Default)]
struct AssignedNames {
    names: BTreeSet<String>,
}

impl<'a> Visit<'a> for AssignedNames {
    fn visit_simple_assignment_target(&mut self, target: &SimpleAssignmentTarget<'a>) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = target {
            self.names.insert(id.name.to_string());
        }
        oxc_ast_visit::walk::walk_simple_assignment_target(self, target);
    }

    fn visit_assignment_target_property_identifier(
        &mut self,
        property: &AssignmentTargetPropertyIdentifier<'a>,
    ) {
        self.names.insert(property.binding.name.to_string());
        oxc_ast_visit::walk::walk_assignment_target_property_identifier(self, property);
    }

    fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
        self.names.insert(id.name.to_string());
    }

    fn visit_function(&mut self, _function: &Function<'a>, _flags: ScopeFlags) {}

    fn visit_arrow_function_expression(&mut self, _arrow: &ArrowFunctionExpression<'a>) {}

    fn visit_class(&mut self, _class: &Class<'a>) {}
}

fn binding_names(pattern: &BindingPattern) -> Vec<String> {
    pattern
        .get_binding_identifiers()
        .iter()
        .map(|id| id.name.to_string())
        .collect()
}

fn declared_names(declaration: &VariableDeclaration) -> Vec<String> {
    declaration
        .declarations
        .iter()
        .flat_map(|declarator| binding_names(&declarator.id))
        .collect()
}

fn statements_of<'p, 'a>(statement: &'p Statement<'a>) -> &'p [Statement<'a>] {
    match statement {
        Statement::BlockStatement(block) => &block.body,
        other => std::slice::from_ref(other),
    }
}

fn is_type_only(statement: &Statement) -> bool {
    matches!(
        statement.as_declaration(),
        Some(
            Declaration::TSTypeAliasDeclaration(_)
                | Declaration::TSInterfaceDeclaration(_)
                | Declaration::TSEnumDeclaration(_)
                | Declaration::TSExternalModuleDeclaration(_)
                | Declaration::TSNamespaceDeclaration(_)
                | Declaration::TSGlobalDeclaration(_)
                | Declaration::TSImportEqualsDeclaration(_)
        )
    )
}

impl<'s> Instrumenter<'s> {
    pub fn new(source: &'s str, lines: &'s LineIndex, edits: &'s mut Edits) -> Instrumenter<'s> {
        Instrumenter {
            source,
            lines,
            edits,
            scopes: Vec::new(),
            analysis: Analysis::default(),
        }
    }

    fn snap(&self) -> String {
        let mut seen = BTreeSet::new();
        let names: Vec<&str> = self
            .scopes
            .iter()
            .flatten()
            .filter(|name| seen.insert(name.as_str()))
            .map(String::as_str)
            .collect();
        format!("() => ({{{}}})", names.join(", "))
    }

    fn probe(&self, line: u32) -> String {
        format!("__xray.line({line}, {}); ", self.snap())
    }

    fn line(&self, offset: u32) -> u32 {
        self.lines.line(offset)
    }

    fn end_line(&self, end: u32) -> u32 {
        self.lines.end_line(end)
    }

    fn declare(&mut self, names: Vec<String>, function_scoped: bool) {
        let scope = if function_scoped {
            self.scopes.first_mut()
        } else {
            self.scopes.last_mut()
        };
        scope.expect("inside a function").extend(names);
    }

    fn declare_variables(&mut self, declaration: &VariableDeclaration) {
        let function_scoped = declaration.kind == VariableDeclarationKind::Var;
        self.declare(declared_names(declaration), function_scoped);
    }

    fn assign(&mut self, line: u32, names: impl IntoIterator<Item = String>) {
        let names: BTreeSet<String> = names.into_iter().collect();
        if !names.is_empty() {
            self.analysis
                .assigned
                .entry(line)
                .or_default()
                .extend(names);
        }
    }

    fn assigned_in(visit: impl FnOnce(&mut AssignedNames)) -> BTreeSet<String> {
        let mut collector = AssignedNames::default();
        visit(&mut collector);
        collector.names
    }

    fn wrap_with_probe(&mut self, test: &Expression, line: u32) {
        let span = test.span();
        let probe = format!("(__xray.line({line}, {}), ", self.snap());
        self.edits.insert(span.start, probe);
        self.edits.insert(span.end, ")".to_string());
    }

    fn body_range(&self, body: &Statement) -> (u32, u32) {
        let statements = statements_of(body);
        match (statements.first(), statements.last()) {
            (Some(first), Some(last)) => (
                self.line(first.span().start),
                self.end_line(last.span().end),
            ),
            _ => (self.line(body.span().start), self.end_line(body.span().end)),
        }
    }

    fn record_condition(&mut self, line: u32, body: &Statement) {
        if statements_of(body).is_empty() {
            return;
        }
        let range = self.body_range(body);
        if range.0 != line {
            self.analysis.conditions.insert(line, range);
        }
    }

    fn record_loop(
        &mut self,
        statement: &Statement,
        body: &Statement,
        targets: Option<Vec<String>>,
    ) {
        let line = self.line(statement.span().start);
        let (body_start, body_end) = self.body_range(body);
        self.analysis.loops.insert(
            line,
            LoopInfo {
                line,
                end_line: self.end_line(statement.span().end),
                body_start,
                body_end,
                targets,
            },
        );
    }

    pub fn function(
        &mut self,
        params: &FormalParameters,
        body_start: u32,
        body_end: u32,
        statements: &[Statement],
    ) {
        self.scopes
            .push(params.iter_bindings().flat_map(binding_names).collect());
        self.edits
            .insert(body_start, format!("__xray.enter({}); try {{", self.snap()));
        self.statements(statements);
        self.edits.insert(
            body_end,
            format!("__xray.ret(undefined, {}); {CATCH}", self.snap()),
        );
        self.scopes.pop();
    }

    pub fn arrow_expression(&mut self, params: &FormalParameters, expression: &Expression) {
        self.scopes
            .push(params.iter_bindings().flat_map(binding_names).collect());
        let span = expression.span();
        let snap = self.snap();
        let line = self.line(span.start);
        self.edits.insert(
            span.start,
            format!(
                "{{ __xray.enter({snap}); try {{ __xray.line({line}, {snap}); return __xray.ret("
            ),
        );
        self.edits
            .insert(span.end, format!(", {snap}); {CATCH} }}"));
        self.scopes.pop();
    }

    fn statements(&mut self, statements: &[Statement]) {
        for statement in statements {
            self.statement(statement);
        }
    }

    fn block(&mut self, block: &BlockStatement) {
        self.scopes.push(Vec::new());
        self.statements(&block.body);
        self.scopes.pop();
    }

    fn body(&mut self, body: &Statement) {
        match body {
            Statement::BlockStatement(block) => self.block(block),
            Statement::EmptyStatement(_) => {}
            other => {
                let span = other.span();
                self.edits.insert(span.start, "{ ".to_string());
                self.scopes.push(Vec::new());
                self.statement(other);
                self.scopes.pop();
                self.edits.insert(span.end, " }".to_string());
            }
        }
    }

    fn body_with_header(&mut self, body: &Statement, line: u32) {
        match body {
            Statement::BlockStatement(block) => {
                self.edits.insert(block.span.start + 1, self.probe(line));
                self.block(block);
            }
            other => {
                let span = other.span();
                self.edits
                    .insert(span.start, format!("{{ {}", self.probe(line)));
                self.scopes.push(Vec::new());
                if !matches!(other, Statement::EmptyStatement(_)) {
                    self.statement(other);
                }
                self.scopes.pop();
                self.edits.insert(span.end, " }".to_string());
            }
        }
    }

    fn needs_probe(statement: &Statement) -> bool {
        !matches!(
            statement,
            Statement::IfStatement(_)
                | Statement::WhileStatement(_)
                | Statement::ForStatement(_)
                | Statement::DoWhileStatement(_)
                | Statement::BlockStatement(_)
                | Statement::TryStatement(_)
                | Statement::EmptyStatement(_)
        ) && !is_type_only(statement)
    }

    fn statement(&mut self, statement: &Statement) {
        let span = statement.span();
        let line = self.line(span.start);
        if Self::needs_probe(statement) {
            self.edits.insert(span.start, self.probe(line));
        }
        self.inner(statement, line);
    }

    fn inner(&mut self, statement: &Statement, line: u32) {
        match statement {
            Statement::IfStatement(branch) => {
                let names = Self::assigned_in(|names| names.visit_expression(&branch.test));
                self.assign(line, names);
                self.wrap_with_probe(&branch.test, line);
                self.record_condition(line, &branch.consequent);
                self.body(&branch.consequent);
                if let Some(alternate) = &branch.alternate {
                    self.body(alternate);
                }
            }
            Statement::WhileStatement(loop_) => {
                let names = Self::assigned_in(|names| names.visit_expression(&loop_.test));
                self.assign(line, names);
                self.wrap_with_probe(&loop_.test, line);
                self.record_condition(line, &loop_.body);
                self.record_loop(statement, &loop_.body, None);
                self.body(&loop_.body);
            }
            Statement::DoWhileStatement(loop_) => {
                self.record_loop(statement, &loop_.body, None);
                self.body_with_header(&loop_.body, line);
            }
            Statement::ForStatement(loop_) => self.for_statement(statement, loop_, line),
            Statement::ForInStatement(loop_) => {
                self.for_each(statement, &loop_.left, &loop_.body, line)
            }
            Statement::ForOfStatement(loop_) => {
                self.for_each(statement, &loop_.left, &loop_.body, line)
            }
            Statement::BlockStatement(block) => self.block(block),
            Statement::LabeledStatement(labeled) => self.inner(&labeled.body, line),
            Statement::SwitchStatement(switch) => {
                let names = Self::assigned_in(|names| names.visit_expression(&switch.discriminant));
                self.assign(line, names);
                self.scopes.push(Vec::new());
                for case in &switch.cases {
                    self.statements(&case.consequent);
                }
                self.scopes.pop();
            }
            Statement::TryStatement(attempt) => {
                self.block(&attempt.block);
                if let Some(handler) = &attempt.handler {
                    let names = handler
                        .param
                        .as_ref()
                        .map(|param| binding_names(&param.pattern))
                        .unwrap_or_default();
                    self.scopes.push(names);
                    self.block(&handler.body);
                    self.scopes.pop();
                }
                if let Some(finalizer) = &attempt.finalizer {
                    self.block(finalizer);
                }
            }
            Statement::ReturnStatement(ret) => {
                let snap = self.snap();
                match &ret.argument {
                    Some(argument) => {
                        let names = Self::assigned_in(|names| names.visit_expression(argument));
                        self.assign(line, names);
                        let span = argument.span();
                        self.edits.insert(span.start, "__xray.ret(".to_string());
                        self.edits.insert(span.end, format!(", {snap})"));
                    }
                    None => self.edits.insert(
                        ret.span.start + "return".len() as u32,
                        format!(" __xray.ret(undefined, {snap})"),
                    ),
                }
            }
            Statement::VariableDeclaration(declaration) => {
                let names =
                    Self::assigned_in(|names| names.visit_variable_declaration(declaration));
                self.assign(line, names);
                self.declare_variables(declaration);
            }
            Statement::FunctionDeclaration(function) => {
                if let Some(id) = &function.id {
                    self.declare(vec![id.name.to_string()], true);
                }
            }
            Statement::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    self.declare(vec![id.name.to_string()], false);
                }
            }
            other => {
                let names = Self::assigned_in(|names| names.visit_statement(other));
                self.assign(line, names);
            }
        }
    }

    fn for_statement(&mut self, statement: &Statement, loop_: &ForStatement, line: u32) {
        self.scopes.push(Vec::new());
        let targets = match &loop_.init {
            Some(ForStatementInit::VariableDeclaration(declaration)) => {
                self.declare_variables(declaration);
                Some(declared_names(declaration))
            }
            Some(init) => {
                let names = Self::assigned_in(|names| names.visit_for_statement_init(init));
                Some(names.into_iter().collect())
            }
            None => None,
        };
        let mut names = Self::assigned_in(|names| {
            if let Some(init) = &loop_.init {
                names.visit_for_statement_init(init);
            }
            if let Some(update) = &loop_.update {
                names.visit_expression(update);
            }
            if let Some(test) = &loop_.test {
                names.visit_expression(test);
            }
        });
        names.extend(targets.iter().flatten().cloned());
        self.assign(line, names);
        if let Some(update) = &loop_.update {
            self.wrap_with_probe(update, line);
        }
        match &loop_.test {
            Some(test) => self.wrap_with_probe(test, line),
            None => {
                let search_from = match &loop_.init {
                    Some(init) => init.span().end,
                    None => statement.span().start,
                } as usize;
                let semicolon = search_from
                    + self.source[search_from..]
                        .find(';')
                        .expect("a for statement has two semicolons");
                self.edits.insert(
                    semicolon as u32 + 1,
                    format!("(__xray.line({line}, {}), true)", self.snap()),
                );
            }
        }
        let targets = targets.filter(|names| !names.is_empty());
        self.record_loop(statement, &loop_.body, targets);
        self.body(&loop_.body);
        self.scopes.pop();
    }

    fn for_each(
        &mut self,
        statement: &Statement,
        left: &ForStatementLeft,
        body: &Statement,
        line: u32,
    ) {
        self.scopes.push(Vec::new());
        let targets: Vec<String> = match left {
            ForStatementLeft::VariableDeclaration(declaration) => {
                self.declare_variables(declaration);
                declared_names(declaration)
            }
            other => Self::assigned_in(|names| names.visit_for_statement_left(other))
                .into_iter()
                .collect(),
        };
        self.assign(line, targets.clone());
        self.record_loop(statement, body, Some(targets));
        self.body_with_header(body, line);
        self.scopes.pop();
    }
}
