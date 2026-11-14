//! `#[derive(Patchable)]` for the state document (spec 01 §3.1).
//!
//! For a struct `Foo` it generates `FooPatch<'a>`, a handle on one value of
//! the document at a JSON Pointer path. Each field gets a setter that changes
//! the value and records an RFC 6902 `replace` operation, so handlers write
//! `tx.tabs().get(&id)?.set_title(t)` instead of building paths by hand.
//!
//! Field attributes:
//! - `#[patch(nested)]`: the field is itself `Patchable`; besides its setter,
//!   `field()` returns its handle.
//! - `#[patch(map)]`: the field is a `BTreeMap`; `field()` returns a
//!   `MapPatch` with `insert`, `remove` and `get`.
//! - `#[patch(skip)]`: no setter (for fields the core manages itself).
//!
//! JSON names follow `#[serde(rename_all = "camelCase")]`, which every state
//! type uses.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, Ident, parse_macro_input};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Value,
    Nested,
    Map,
    Skip,
}

fn kind_of(field: &syn::Field) -> syn::Result<Kind> {
    let mut kind = Kind::Value;
    for attr in &field.attrs {
        if !attr.path().is_ident("patch") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            kind = if meta.path.is_ident("nested") {
                Kind::Nested
            } else if meta.path.is_ident("map") {
                Kind::Map
            } else if meta.path.is_ident("skip") {
                Kind::Skip
            } else {
                return Err(meta.error("expected `nested`, `map` or `skip`"));
            };
            Ok(())
        })?;
    }
    Ok(kind)
}

/// snake_case to camelCase, as serde's `rename_all = "camelCase"` does.
fn camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = false;
    for c in name.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// `r#type` becomes `type` in method names and JSON.
fn bare(ident: &Ident) -> String {
    ident.to_string().trim_start_matches("r#").to_owned()
}

/// `field()` on a nested `Patchable` field: a handle one level down.
fn nested_getter(
    ident: &Ident,
    ty: &syn::Type,
    raw: &str,
    segment: &str,
) -> proc_macro2::TokenStream {
    let getter = Ident::new(raw, Span::call_site());
    quote! {
        pub fn #getter(&mut self) -> <#ty as ::mado_types::patch::Patchable>::Patch<'_> {
            let path = ::std::format!("{}{}", self.path, #segment);
            ::mado_types::patch::Patchable::patch(&mut self.target.#ident, path, self.recorder)
        }
    }
}

#[proc_macro_derive(Patchable, attributes(patch))]
pub fn derive_patchable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            name,
            "Patchable derives only for structs; enums are replaced whole",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            name,
            "Patchable needs named fields",
        ));
    };
    let handle = format_ident!("{}Patch", name);
    let vis = &input.vis;
    let mut methods = Vec::new();
    for field in &fields.named {
        let Some(ident) = &field.ident else { continue };
        let ty = &field.ty;
        let raw = bare(ident);
        let json = camel(&raw);
        let segment = format!("/{json}");
        let kind = kind_of(field)?;
        match kind {
            Kind::Skip => {}
            Kind::Value | Kind::Nested => {
                let setter = Ident::new(&format!("set_{raw}"), Span::call_site());
                let doc = format!("Sets `{json}`, recording a `replace` when the value changes.");
                methods.push(quote! {
                    #[doc = #doc]
                    pub fn #setter(&mut self, value: #ty) -> &mut Self {
                        if self.target.#ident != value {
                            let path = ::std::format!("{}{}", self.path, #segment);
                            self.recorder.replace(path, &value);
                            self.target.#ident = value;
                        }
                        self
                    }
                });
                if kind == Kind::Nested {
                    methods.push(nested_getter(ident, ty, &raw, &segment));
                }
            }
            Kind::Map => {
                let getter = Ident::new(&raw, Span::call_site());
                methods.push(quote! {
                    pub fn #getter(&mut self) -> ::mado_types::patch::MapPatch<'_, #ty> {
                        let path = ::std::format!("{}{}", self.path, #segment);
                        ::mado_types::patch::MapPatch::new(&mut self.target.#ident, path, self.recorder)
                    }
                });
            }
        }
    }
    let doc = format!(
        "A handle on one `{name}` in the state document. Generated by `#[derive(Patchable)]`."
    );
    Ok(quote! {
        #[doc = #doc]
        #[derive(Debug)]
        #vis struct #handle<'a> {
            target: &'a mut #name,
            path: ::std::string::String,
            recorder: &'a mut ::mado_types::patch::PatchRecorder,
        }

        impl<'a> #handle<'a> {
            /// The value this handle edits, for reading.
            pub fn get(&self) -> &#name {
                self.target
            }

            #(#methods)*
        }

        impl ::mado_types::patch::Patchable for #name {
            type Patch<'a> = #handle<'a>;

            fn patch<'a>(
                &'a mut self,
                path: ::std::string::String,
                recorder: &'a mut ::mado_types::patch::PatchRecorder,
            ) -> #handle<'a> {
                #handle { target: self, path, recorder }
            }
        }
    })
}
