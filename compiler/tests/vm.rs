#[cfg(test)]
mod test {

    use compiler_lib::modules::lexer::lexer;
    use compiler_lib::modules::parser::Parser;
    use compiler_lib::modules::vm::VM;

    #[derive(serde::Deserialize)]
    struct Case {
        src: String,
        output: Vec<String>,
        result: String,
        #[serde(default)]
        error: Option<String>,
    }

    #[test]
    fn test_cases() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/vm.json")).expect("invalid JSON");

        for case in cases {
            let (chunk, _errors) = Parser::new(&case.src, lexer(&case.src)).parse();
            let mut vm = VM::new(&chunk);
            let result = vm.run();

            match result {
                Ok(obj) => {
                    assert_eq!(vm.display(obj), case.result, "result mismatch on: {:?}", case.src);
                    assert_eq!(vm.output, case.output, "output mismatch on: {:?}", case.src);
                }
                Err(e) => match &case.error {
                    Some(expected) => assert!(
                        e.to_string().contains(expected.as_str()),
                        "wrong error on {:?}: got '{}', expected '{}'", case.src, e, expected
                    ),
                    None => panic!("VM error on {:?}: {}", case.src, e),
                }
            }
        }
    }
}