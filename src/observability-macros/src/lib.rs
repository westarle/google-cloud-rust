use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, Data, DeriveInput, Expr, Fields, Ident, Lit, Meta, Type, TypePath,
};

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

    let mut key_consts = Vec::new();

    for field in fields {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;

        for attr in &field.attrs {
            if attr.path().is_ident("observability") {
                let mut key_found = false;
                if let Ok(list) = attr.parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated) {
                    for meta in list {
                        if let Meta::NameValue(nv) = meta {
                            if nv.path.is_ident("key") {
                                key_found = true;
                                if let Expr::Lit(expr_lit) = nv.value {
                                    if let Lit::Str(lit_str) = expr_lit.lit {
                                        let key_name = lit_str.value();
                                        let const_ident = Ident::new(
                                            &format!("KEY_{}", field_name.to_string().to_uppercase()),
                                            field_name.span(),
                                        );

                                        if is_supported_type(field_type) {
                                            key_consts.push(quote! {
                                                const #const_ident: &'static str = #key_name;
                                            });
                                        } else {
                                            panic!("Unsupported field type for observability key on field: {}", field_name);
                                        }
                                    }
                                } else {
                                    panic!("Observability key must be a string literal on field: {}", field_name);
                                }
                            }
                        }
                    }
                }
                if !key_found {
                    panic!("Missing 'key' argument in #[observability] attribute on field: {}", field_name);
                }
            }
        }
    }

    let expanded = quote! {
        impl #struct_name {
            #(#key_consts)*

            pub fn create_span(&self) -> ::tracing::Span {
                ::tracing::info_span!(#span_name_str)
            }

            pub fn record_response_attributes(&self, _record: &mut ::tracing::span::Record<'_>) {
                // Placeholder
            }
        }
    };

    TokenStream::from(expanded)
}

fn is_supported_type(ty: &Type) -> bool {
    match ty {
        Type::Path(TypePath { path, .. }) => {
            if let Some(ident) = path.get_ident() {
                return ident == "String" || ident == "i64";
            }
            // Handle Option<String> and Option<i64>
            if path.segments.len() == 1 && path.segments[0].ident == "Option" {
                if let syn::PathArguments::AngleBracketed(args) = &path.segments[0].arguments {
                    if args.args.len() == 1 {
                        if let syn::GenericArgument::Type(inner_ty) = &args.args[0] {
                            return is_supported_type(inner_ty);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    false
}
