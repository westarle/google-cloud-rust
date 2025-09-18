use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, Data, DeriveInput, Expr, Fields, Ident, Lit, Meta, Type, TypePath,
};

struct FieldInfo<'a> {
    field_name: &'a Ident,
    field_type: &'a Type,
    key_name: String,
    const_ident: Ident,
    is_response_phase: bool,
}

#[proc_macro_derive(ObservabilityAttributes, attributes(observability))]
pub fn derive_observability_attributes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let struct_name = &input.ident;

    let mut span_name_str = struct_name.to_string();

    for attr in &input.attrs {
        if attr.path().is_ident("observability") {
            if let Ok(list) = attr.parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated) {
                for meta in list {
                    if let Meta::NameValue(nv) = meta {
                        if nv.path.is_ident("name") {
                            if let Expr::Lit(expr_lit) = nv.value {
                                if let Lit::Str(lit_str) = expr_lit.lit {
                                    span_name_str = lit_str.value();
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            _ => panic!("Only structs with named fields are supported"),
        },
        _ => panic!("Only structs are supported"),
    };

    let mut field_infos = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;
        let mut key_name = None;
        let mut is_response_phase = false;

        let mut has_observability_attr = false;
        for attr in &field.attrs {
            if attr.path().is_ident("observability") {
                has_observability_attr = true;
                if let Ok(list) = attr.parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated) {
                    for meta in list {
                        if let Meta::NameValue(nv) = meta {
                            if nv.path.is_ident("key") {
                                if let Expr::Lit(expr_lit) = nv.value {
                                    if let Lit::Str(lit_str) = expr_lit.lit {
                                        key_name = Some(lit_str.value());
                                    } else {
                                        panic!("Observability key must be a string literal on field: {}", field_name);
                                    }
                                } else {
                                    panic!("Observability key must be a string literal on field: {}", field_name);
                                }
                            } else if nv.path.is_ident("phase") {
                                if let Expr::Lit(expr_lit) = nv.value {
                                    if let Lit::Str(lit_str) = expr_lit.lit {
                                        if lit_str.value() == "response" {
                                            is_response_phase = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if has_observability_attr && key_name.is_none() {
            panic!("Missing 'key' argument in #[observability] attribute on field: {}", field_name);
        }

        if let Some(key_name_val) = key_name {
            if !is_supported_type(field_type) {
                panic!("Unsupported field type for observability key on field: {}. Only String, i64, Option<String>, and Option<i64> are supported.", field_name);
            }
            let const_ident = Ident::new(
                &format!("KEY_{}", field_name.to_string().to_uppercase()),
                field_name.span(),
            );
            field_infos.push(FieldInfo {
                field_name,
                field_type,
                key_name: key_name_val,
                const_ident,
                is_response_phase,
            });
        } else {
            // Fields without #[observability(key = "...")] are allowed, just ignored by the macro
        }
    }

    let key_consts = field_infos.iter().map(|info| {
        let const_ident = &info.const_ident;
        let key_name = &info.key_name;
        quote! {
            const #const_ident: &'static str = #key_name;
        }
    });

    let create_span_attrs = field_infos.iter().map(|info| {
        let field_name = info.field_name;
        let field_type = info.field_type;
        let const_ident = &info.const_ident;

        if is_option_type(field_type) {
            let inner_ty = get_option_inner_type(field_type).unwrap();
            if is_string_type(inner_ty) {
                quote! {
                    { #struct_name::#const_ident } = self.#field_name.as_deref()
                }
            } else if is_i64_type(inner_ty) {
                quote! {
                    { #struct_name::#const_ident } = self.#field_name
                }
            } else {
                unreachable!(); // Should be caught by is_supported_type
            }
        } else if is_string_type(field_type) {
            quote! {
                { #struct_name::#const_ident } = self.#field_name.as_str()
            }
        } else if is_i64_type(field_type) {
            quote! {
                { #struct_name::#const_ident } = self.#field_name
            }
        } else {
            unreachable!(); // Should be caught by is_supported_type
        }
    });

    let expanded = quote! {
        impl #struct_name {
            #(#key_consts)*

            pub fn create_span(&self) -> ::tracing::Span {
                ::tracing::info_span!(
                    #span_name_str,
                    #(#create_span_attrs),*
                )
            }

            pub fn record_response_attributes(&self, _record: &mut ::tracing::span::Record<'_>) {
                // Placeholder
            }
        }
    };

    TokenStream::from(expanded)
}

fn is_string_type(ty: &Type) -> bool {
    match ty {
        Type::Path(TypePath { path, .. }) => path.get_ident().map_or(false, |id| id == "String"),
        _ => false,
    }
}

fn is_i64_type(ty: &Type) -> bool {
    match ty {
        Type::Path(TypePath { path, .. }) => path.get_ident().map_or(false, |id| id == "i64"),
        _ => false,
    }
}

fn get_option_inner_type(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Path(TypePath { path, .. }) => {
            if path.segments.len() == 1 && path.segments[0].ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &path.segments[0].arguments {
                    if args.args.len() == 1 {
                        if let syn::GenericArgument::Type(inner_ty) = &args.args[0] {
                            return Some(inner_ty);
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn is_supported_type(ty: &Type) -> bool {
    is_string_type(ty) || is_i64_type(ty) ||
    (is_option_type(ty) && get_option_inner_type(ty).map_or(false, |inner| is_string_type(inner) || is_i64_type(inner)))
}

fn is_option_type(ty: &Type) -> bool {
    match ty {
        Type::Path(TypePath { path, .. }) => {
            path.segments.last().map_or(false, |seg| seg.ident == "Option")
        }
        _ => false,
    }
}