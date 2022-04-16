extern crate proc_macro;
use proc_macro::TokenStream;
use syn::{parse_macro_input, DeriveInput, Error};

#[proc_macro_derive(CommandArgsBlock, attributes(argtoken))]
pub fn derive_command_args_block(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    expand::expand(input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

mod expand {
    use proc_macro2::{Span, TokenStream};
    use quote::{quote, quote_spanned};
    use syn::{
        parse_quote, spanned::Spanned, DeriveInput, Error, GenericParam, Lifetime, LifetimeDef,
        LitStr, Path, Result, Type, PathArguments, GenericArgument, TypePath, PathSegment,
    };

    pub(crate) fn expand(input: DeriveInput) -> Result<TokenStream> {
        let default_lifetime = LifetimeDef::new(Lifetime::new("'a", Span::call_site()));
        let mut impl_generics = input.generics.clone();
        if impl_generics.lifetimes().next().is_none() {
            impl_generics
                .params
                .push(GenericParam::Lifetime(default_lifetime));
        }
        let lifetime = &impl_generics.lifetimes().next().unwrap().lifetime;
        let (_, ty_generics, where_clause) = input.generics.split_for_impl();
        let (impl_generics_tok, _, _) = impl_generics.split_for_impl();

        let token_path: Path = parse_quote!(argtoken);
        let token = input
            .attrs
            .iter()
            .find(|a| a.path == token_path)
            .map(|a| a.parse_args::<LitStr>())
            .transpose()?;
        let name = &input.ident;
        let parse_fn_content = parse_fn_content(&input, &token)?;
        let parse_maybe_fn_content = parse_maybe_fn_content(&input, &token)?;
        let parse_fn = quote! {
            fn parse(args: &mut &[&#lifetime str]) -> Result<Self, ::command_args::Error> {
                #parse_fn_content
            }
            fn parse_maybe(args: &mut &[&#lifetime str]) -> Result<Option<Self>, ::command_args::Error> {
                #parse_maybe_fn_content
            }
        };

        Ok(quote! {
            impl #impl_generics_tok ::command_args::CommandArgs<#lifetime> for #name #ty_generics #where_clause {
                #parse_fn
            }
        })
    }

    fn parse_fn_content(input: &DeriveInput, token: &Option<LitStr>) -> Result<TokenStream> {
        let parse_token = if let Some(token) = token {
            let token_span = token.span();
            quote_spanned! {token_span=>
                if let Some(&#token) = args.get(0) {
                    *args = &mut &args[1..];
                } else {
                    return Err(::command_args::Error::TokenNotFound(#token));
                }
            }
        } else {
            TokenStream::new()
        };
        let (parse_fields, returned_struct) = fields_parse(input)?;
        Ok(quote! {
            #parse_token
            #parse_fields
            Ok(#returned_struct)
        })
    }

    fn parse_maybe_fn_content(input: &DeriveInput, token: &Option<LitStr>) -> Result<TokenStream> {
        let parse_token = if let Some(token) = token {
            let token_span = token.span();
            quote_spanned! {token_span=>
                if let Some(&#token) = args.get(0) {
                    *args = &mut &args[1..];
                } else {
                    return Ok(None);
                }
            }
        } else {
            // Without token, if args is empty => None
            let span = input.span();
            quote_spanned! {span=>
                if args.is_empty() {
                    return Ok(None);
                }
            }
        };
        let (parse_fields, returned_struct) = fields_parse(input)?;
        Ok(quote! {
            #parse_token
            #parse_fields
            Ok(Some(#returned_struct))
        })
    }

    fn fields_parse(input: &DeriveInput) -> Result<(TokenStream, TokenStream)> {
        match &input.data {
            syn::Data::Struct(s) => Ok(struct_fields_parse(s)?),
            syn::Data::Enum(_) => {
                return Err(Error::new(input.span(), "enum not supported"));
            }
            syn::Data::Union(_) => {
                return Err(Error::new(input.span(), "union not supported"));
            }
        }
    }

    fn struct_fields_parse(s: &syn::DataStruct) -> Result<(TokenStream, TokenStream)> {
        match &s.fields {
            syn::Fields::Named(named) => named_fields_parse(named),
            syn::Fields::Unnamed(tuple) => tuple_fields_parse(tuple),
            syn::Fields::Unit => {
                return Err(Error::new(
                    s.struct_token.span(),
                    "unit struct not supported",
                ));
            }
        }
    }

    fn tuple_fields_parse(_tuple: &syn::FieldsUnnamed) -> Result<(TokenStream, TokenStream)> {
        todo!()
    }

    fn last_path_segment(ty: &Type) -> Option<&PathSegment> {
        match ty {
            &Type::Path(TypePath {
                qself: None,
                path:
                    Path {
                        segments: ref seg,
                        leading_colon: _,
                    },
            }) => seg.last(),
            _ => None,
        }
    }

    // if this type is Option and return the Wrapped type
    fn option_inner_type(ty: &Type) -> Option<&GenericArgument> {
        match last_path_segment(&ty) {
            Some(PathSegment {
                ident,
                arguments: PathArguments::AngleBracketed(ref gen_arg),
            }) if ident == "Option" => gen_arg.args.first(),
            _ => None,
        }
    }

    fn named_fields_parse(named: &syn::FieldsNamed) -> Result<(TokenStream, TokenStream)> {
        let declare_vars = named.named.iter().map(|f| {
            let ty = &f.ty;
            let ty_span = f.ty.span();
            let var_name = f.ident.as_ref().unwrap();

            match option_inner_type(ty) {
                Some(inner_ty) => quote_spanned!{ty_span=>
                    let #var_name = <#inner_ty as ::command_args::CommandArgs>::parse_maybe(args)?;
                },
                None => quote_spanned! {ty_span=>
                    let #var_name = <#ty as ::command_args::CommandArgs>::parse(args)?;
                },
            }
        });
        let return_fields = named.named.iter().map(|f| {
            f.ident.as_ref()
        });

        Ok((quote!(#(#declare_vars)*), quote!(Self { #(#return_fields),* })))
    }
}