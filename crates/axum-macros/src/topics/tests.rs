use syn::{LitStr, parse_quote};

use overseerd_macros_core::paths::Paths;

use super::{Segment, TopicsArgs, expand, parse_template};

fn lit(template: &str) -> LitStr {
    syn::parse_str(&format!("{template:?}")).expect("valid string literal")
}

#[test]
fn literal_only_template_is_one_segment() {
    let segments = parse_template(&lit("/topic/chat")).unwrap();

    assert_eq!(segments, vec![Segment::Literal("/topic/chat".to_string())]);
}

#[test]
fn hole_is_recognized_between_literals() {
    let segments = parse_template(&lit("/topic/{room}/history")).unwrap();

    assert_eq!(
        segments,
        vec![
            Segment::Literal("/topic/".to_string()),
            Segment::Hole("room".to_string()),
            Segment::Literal("/history".to_string()),
        ]
    );
}

#[test]
fn doubled_braces_are_literal() {
    let segments = parse_template(&lit("/topic/{{literal}}")).unwrap();

    assert_eq!(
        segments,
        vec![Segment::Literal("/topic/{literal}".to_string())]
    );
}

/// Regression test: a hole missing its closing `}` must be rejected rather than silently
/// treated as if it extended to the end of the template.
#[test]
fn unclosed_hole_is_rejected() {
    let result = parse_template(&lit("/topic/{room"));

    assert!(result.is_err());
}

#[test]
fn empty_hole_is_rejected() {
    let result = parse_template(&lit("/topic/{}"));

    assert!(result.is_err());
}

#[test]
fn unmatched_closing_brace_is_rejected() {
    let result = parse_template(&lit("/topic/}"));

    assert!(result.is_err());
}

#[test]
fn topics_require_protocol() {
    let result = syn::parse_str::<TopicsArgs>("");

    assert!(result.is_err());
    assert!(result.err().unwrap().to_string().contains("protocol = P"));
}

#[test]
fn topics_use_explicit_protocol_and_default_codec() {
    let args = syn::parse_str::<TopicsArgs>("protocol = custom::Wire").expect("topics args");
    let item = parse_quote! {
        enum Events {
            #[topic("events")]
            Changed(Change),
        }
    };
    let paths = Paths::new(parse_quote!(::core_crate), parse_quote!(::plugin_crate));
    let tokens = expand(args, item, &paths)
        .expect("topics expansion")
        .to_string();

    assert!(tokens.contains("type Protocol = custom :: Wire"));
    assert!(
        tokens
            .contains("< custom :: Wire as :: plugin_crate :: MessagingProtocol > :: DefaultCodec")
    );
}
