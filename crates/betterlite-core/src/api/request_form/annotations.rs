use indexmap::IndexMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumericType {
    Int,
    Float,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Qualifier {
    Annotated,
    NotRequired,
    ReadOnly,
    Required,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormAnnotation {
    Int,
    Float,
    NoneType,
    Union(Vec<FormAnnotation>),
    Qualified(Qualifier, Box<FormAnnotation>),
    Other,
}

pub type FormAnnotations = IndexMap<String, FormAnnotation>;

#[must_use]
pub fn numeric_form_fields(annotations: &FormAnnotations) -> IndexMap<String, NumericType> {
    annotations
        .iter()
        .filter_map(|(name, annotation)| {
            let scalar = numeric_form_type(annotation)?;
            Some((name.clone(), scalar))
        })
        .collect()
}

#[must_use]
pub fn numeric_form_type(annotation: &FormAnnotation) -> Option<NumericType> {
    let unwrapped = unqualified(annotation);
    let FormAnnotation::Union(members) = unwrapped else {
        return scalar_type(unwrapped);
    };
    let mut present = members
        .iter()
        .filter(|member| !matches!(member, FormAnnotation::NoneType));
    let first = present.next()?;
    if present.next().is_some() {
        return None;
    }
    scalar_type(first)
}

fn scalar_type(annotation: &FormAnnotation) -> Option<NumericType> {
    match annotation {
        FormAnnotation::Int => Some(NumericType::Int),
        FormAnnotation::Float => Some(NumericType::Float),
        FormAnnotation::NoneType
        | FormAnnotation::Union(_)
        | FormAnnotation::Qualified(_, _)
        | FormAnnotation::Other => None,
    }
}

fn unqualified(mut annotation: &FormAnnotation) -> &FormAnnotation {
    while let FormAnnotation::Qualified(_, inner) = annotation {
        annotation = inner;
    }
    annotation
}
