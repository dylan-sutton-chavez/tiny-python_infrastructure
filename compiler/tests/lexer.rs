#[cfg(test)]
mod test {

    use compiler_lib::modules::lexer::lexer;

    #[derive(serde::Deserialize)]
    struct Case {
        src: String,
        tokens: Vec<String>,
    }

    #[test]
    fn test_cases() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/lexer.json")).expect("invalid JSON");

        for case in cases {
            let got: Vec<String> = lexer(&case.src).map(|t| format!("{:?}", t.kind)).collect();
            assert_eq!(got, case.tokens, "failed on: {:?}", case.src);
        }
    }
}
