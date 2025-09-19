use proc_macro::TokenStream;
use quote::quote;
use std::collections::{HashMap, HashSet};
use syn::{    parse::{Parse, ParseStream},    parse_macro_input,    spanned::Spanned,    visit::{self, Visit},    Fields, Ident, ItemConst, ItemMod, ItemStruct, Meta, Result, Token, Type,};

struct MacroArgs {
    struct_name: Ident,
    key_prefix: String,
}

impl Parse for MacroArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut struct_name = None;
        let mut key_prefix = "KEY".to_string();

        let metas = input.parse_terminated(Meta::parse, Token![,])?;

        for meta in metas {
            match meta {
                Meta::NameValue(nv) => {
                    if nv.path.is_ident("struct_name") {
                        if let syn::Expr::Lit(ref expr_lit) = nv.value {
                            if let syn::Lit::Str(ref s) = expr_lit.lit {
                                if struct_name.is_some() {
                                    return Err(syn::Error::new_spanned(
                                        &nv.path,
                                        "Duplicate struct_name argument",
                                    ));
                                }
                                struct_name = Some(s.parse()?);
                            } else {
                                return Err(syn::Error::new_spanned(
                                    &nv.value,
                                    "Expected string literal for struct_name",
                                ));
                            }
                        } else {
                            return Err(syn::Error::new_spanned(
                                &nv.value,
                                "Expected literal for struct_name",
                            ));
                        }
                    } else if nv.path.is_ident("key_prefix") {
                        if let syn::Expr::Lit(ref expr_lit) = nv.value {
                            if let syn::Lit::Str(ref s) = expr_lit.lit {
                                key_prefix = s.value();
                            } else {
                                return Err(syn::Error::new_spanned(
                                    &nv.value,
                                    "Expected string literal for key_prefix",
                                ));
                            }
                        } else {
                            return Err(syn::Error::new_spanned(
                                &nv.value,
                                "Expected literal for key_prefix",
                            ));
                        }
                    } else {
                        return Err(syn::Error::new_spanned(&nv.path, "Unknown argument"));
                    }
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        &meta,
                        "Expected name-value argument, e.g., struct_name = \"MyStruct\"",
                    ))
                }
            }
        }

        let struct_name = struct_name.ok_or_else(|| {
            syn::Error::new(input.span(), "Missing required argument: struct_name")
        })?;

        Ok(MacroArgs {
            struct_name,
            key_prefix,
        })
    }
}

struct ModVisitor<'a> {
    args: &'a MacroArgs,
    target_struct: Option<&'a ItemStruct>,
    key_consts: Vec<&'a ItemConst>,
    errors: Vec<syn::Error>,
}

impl<'a> ModVisitor<'a> {
    fn new(args: &'a MacroArgs) -> Self {
        ModVisitor {
            args,
            target_struct: None,
            key_consts: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl<'ast> Visit<'ast> for ModVisitor<'ast> {
    fn visit_item_struct(&mut self, i: &'ast ItemStruct) {
        if i.ident == self.args.struct_name {
            if self.target_struct.is_some() {
                self.errors.push(syn::Error::new_spanned(
                    &i.ident,
                    format!("Duplicate struct definition for {}", self.args.struct_name),
                ));
            } else {
                self.target_struct = Some(i);
            }
        }
        visit::visit_item_struct(self, i);
    }

    fn visit_item_const(&mut self, i: &'ast ItemConst) {
        let const_name = i.ident.to_string();
        let prefix = format!("{}_", self.args.key_prefix);
        if const_name.starts_with(&prefix) {
            // Check if the type is &str
            if let Type::Reference(ref type_ref) = *i.ty {
                if let Type::Path(ref type_path) = *type_ref.elem {
                    if type_path.path.is_ident("str") {
                        self.key_consts.push(i);
                    } else {
                        self.errors.push(syn::Error::new_spanned(
                            &i.ty,
                            format!("Key const {} must be of type &str", const_name),
                        ));
                    }
                } else {
                    self.errors.push(syn::Error::new_spanned(
                        &i.ty,
                        format!("Key const {} must be of type &str", const_name),
                    ));
                }
            } else {
                self.errors.push(syn::Error::new_spanned(
                    &i.ty,
                    format!("Key const {} must be of type &str", const_name),
                ));
            }
        }
        visit::visit_item_const(self, i);
    }
}

fn normalize_key_to_field(key_suffix: &str) -> String {
    key_suffix.to_lowercase()
}

#[proc_macro_attribute]
pub fn ensure_keys_and_fields_in_sync(args: TokenStream, input: TokenStream) -> TokenStream {
    let args_span = proc_macro2::TokenStream::from(args.clone()).span();
    let macro_args = parse_macro_input!(args as MacroArgs);
    let input_mod = parse_macro_input!(input as ItemMod);

    if input_mod.content.is_none() {
        return syn::Error::new_spanned(input_mod.mod_token, "Module must have a body")
            .to_compile_error()
            .into();
    }
    let items = &input_mod.content.as_ref().unwrap().1;

    let mut visitor = ModVisitor::new(&macro_args);
    for item in items {
        visitor.visit_item(item);
    }

    if !visitor.errors.is_empty() {
        let mut combined_error = visitor.errors.remove(0);
        for err in visitor.errors {
            combined_error.combine(err);
        }
        return combined_error.to_compile_error().into();
    }

    let target_struct = match visitor.target_struct {
        Some(s) => s,
        None => {
            return syn::Error::new_spanned(
                &macro_args.struct_name,
                format!("Struct {} not found in module", macro_args.struct_name),
            )
            .to_compile_error()
            .into();
        }
    };

    if visitor.key_consts.is_empty() {
        return syn::Error::new(
            args_span,
            format!(
                "No key consts found starting with '{}_' in module '{}'",
                macro_args.key_prefix, input_mod.ident
            ),
        )
        .to_compile_error()
        .into();
    }

    // Pass 2: Data Extraction
    let mut field_names = HashSet::new();
    if let Fields::Named(ref fields_named) = target_struct.fields {
        for field in &fields_named.named {
            if let Some(ident) = &field.ident {
                field_names.insert(ident.to_string());
            }
        }
    } else {
        return syn::Error::new_spanned(target_struct, "Target struct must have named fields")
            .to_compile_error()
            .into();
    }

    let prefix = format!("{}_", macro_args.key_prefix);
    let mut defined_keys = HashMap::new(); // Map normalized field name to const Ident
    for key_const in &visitor.key_consts {
        let const_name = key_const.ident.to_string();
        let suffix = const_name.trim_start_matches(&prefix);
        let field_name = normalize_key_to_field(suffix);
        defined_keys.insert(field_name, key_const.ident.clone());
    }

    // Pass 3: Verification
    let mut errors = Vec::<syn::Error>::new();

    // Check for keys without corresponding fields
    for (field_name, key_ident) in &defined_keys {
        if !field_names.contains(field_name) {
            errors.push(syn::Error::new_spanned(
                key_ident,
                format!(
                    "Key const {} has no corresponding field '{}' in struct {}",
                    key_ident, field_name, macro_args.struct_name
                ),
            ));
        }
    }

    // Check for fields without corresponding keys
    if let Fields::Named(ref fields_named) = target_struct.fields {
        for field in &fields_named.named {
            if let Some(field_ident) = &field.ident {
                let field_name = field_ident.to_string();
                if !defined_keys.contains_key(&field_name) {
                     let expected_key_suffix = field_name.to_uppercase();
                     let expected_key_name = format!("{}{}", prefix, expected_key_suffix);
                     errors.push(syn::Error::new_spanned(
                        field_ident,
                        format!(
                            "Field '{}' in struct {} has no corresponding key const named {}",
                            field_name, macro_args.struct_name, expected_key_name
                        ),
                    ));
                }
            }
        }
    }

    if !errors.is_empty() {
        let mut combined_error = errors.remove(0);
        for err in errors {
            combined_error.combine(err);
        }
        return combined_error.to_compile_error().into();
    }

    // If all checks pass, return the original module
    TokenStream::from(quote! {
        #input_mod
    })
}