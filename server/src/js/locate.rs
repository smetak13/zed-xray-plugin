use oxc_ast::ast::{
    ArrowFunctionExpression, CallExpression, Declaration, Expression, FormalParameters, Function,
    Program, Statement,
};
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, Span};

pub enum Body<'p, 'a> {
    Function(&'p Function<'a>),
    Arrow(&'p ArrowFunctionExpression<'a>),
}

pub struct Target<'p, 'a> {
    pub name: String,
    pub statement: Span,
    pub body: Body<'p, 'a>,
}

impl<'p, 'a> Target<'p, 'a> {
    pub fn params(&self) -> &'p FormalParameters<'a> {
        match self.body {
            Body::Function(function) => &function.params,
            Body::Arrow(arrow) => &arrow.params,
        }
    }

    pub fn is_async_or_generator(&self) -> bool {
        match self.body {
            Body::Function(function) => function.r#async || function.generator,
            Body::Arrow(arrow) => arrow.r#async,
        }
    }
}

fn declared_functions<'p, 'a>(statement: &'p Statement<'a>) -> Vec<(String, Body<'p, 'a>)> {
    let declaration = match statement {
        Statement::ExportDeclaration(export) => &export.declaration,
        other => match other.as_declaration() {
            Some(declaration) => declaration,
            None => return Vec::new(),
        },
    };
    match declaration {
        Declaration::FunctionDeclaration(function) => function
            .id
            .iter()
            .map(|id| (id.name.to_string(), Body::Function(function)))
            .collect(),
        Declaration::VariableDeclaration(variables) => variables
            .declarations
            .iter()
            .filter_map(|declarator| {
                let name = declarator.id.get_identifier_name()?.to_string();
                match declarator.init.as_ref()? {
                    Expression::FunctionExpression(function) => {
                        Some((name, Body::Function(function)))
                    }
                    Expression::ArrowFunctionExpression(arrow) => Some((name, Body::Arrow(arrow))),
                    _ => None,
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

pub fn find_function<'p, 'a>(
    program: &'p Program<'a>,
    contains: impl Fn(Span) -> bool,
    name: Option<&str>,
) -> Option<Target<'p, 'a>> {
    let targets: Vec<Target> = program
        .body
        .iter()
        .flat_map(|statement| {
            declared_functions(statement)
                .into_iter()
                .map(|(name, body)| Target {
                    name,
                    statement: statement.span(),
                    body,
                })
        })
        .collect();
    let at_cursor = targets.iter().position(|target| contains(target.statement));
    let by_name = || {
        targets
            .iter()
            .position(|target| Some(target.name.as_str()) == name)
    };
    let index = at_cursor.or_else(by_name)?;
    targets.into_iter().nth(index)
}

struct CallFinder<'n> {
    name: &'n str,
    found: bool,
}

impl<'a> Visit<'a> for CallFinder<'_> {
    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if matches!(&call.callee, Expression::Identifier(id) if id.name == self.name) {
            self.found = true;
        }
        oxc_ast_visit::walk::walk_call_expression(self, call);
    }
}

pub fn find_call<'p, 'a>(program: &'p Program<'a>, target: &Target) -> Option<&'p Statement<'a>> {
    program.body.iter().find(|statement| {
        if statement.span() == target.statement {
            return false;
        }
        let mut finder = CallFinder {
            name: &target.name,
            found: false,
        };
        finder.visit_statement(statement);
        finder.found
    })
}

pub fn call_template(target: &Target) -> String {
    let params = target.params();
    let names: Vec<String> = params
        .items
        .iter()
        .map(|param| {
            param
                .pattern
                .get_identifier_name()
                .map(|name| name.to_string())
        })
        .chain(params.rest.iter().map(|rest| {
            rest.rest
                .argument
                .get_identifier_name()
                .map(|name| name.to_string())
        }))
        .enumerate()
        .map(|(index, name)| name.unwrap_or_else(|| format!("arg{}", index + 1)))
        .collect();
    let placeholders: Vec<String> = names.iter().map(|name| format!("/* {name} */")).collect();
    format!("{}({});", target.name, placeholders.join(", "))
}
