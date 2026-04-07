// lexer/mod.rs

pub mod tables;
mod scan;

use scan::Scanner;

const MAX_SOURCE_SIZE: usize = 10 * 1024 * 1024;

/*
Token
    Token kind with line number and byte-offset span into source.
*/

#[derive(Debug)]
pub struct Token {
    pub kind: TokenType,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/*
Token Types
    Enumeration of all lexical tokens produced by the scanner.
*/

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum TokenType {
    // Keywords
    False, None, True, And, As, Assert, Async, Await, Break, Class, Continue, Def, Del,
    Elif, Else, Except, Finally, For, From, Global, If, Import, In, Is, Lambda, Nonlocal,
    Not, Or, Pass, Raise, Return, Try, While, With, Yield,
    // Soft keywords
    Case, Match, Type, Underscore,
    // Operators (3-char)
    DoubleStarEqual, DoubleSlashEqual, LeftShiftEqual, RightShiftEqual,
    // Operators (2-char)
    NotEqual, PercentEqual, AmperEqual, DoubleStar, StarEqual, PlusEqual, MinEqual,
    Rarrow, Ellipsis, DoubleSlash, SlashEqual, ColonEqual, LeftShift, LessEqual,
    EqEqual, GreaterEqual, RightShift, AtEqual, CircumflexEqual, VbarEqual,
    // Operators (1-char)
    Exclamation, Percent, Amper, Star, Plus, Minus, Dot, Slash, Less, Equal, Greater,
    At, Circumflex, Vbar, Tilde, Comma, Colon, Semi,
    // Delimiters
    Lpar, Rpar, Lsqb, Rsqb, Lbrace, Rbrace,
    // Literals
    Name, Complex, Float, Int, String,
    // F-string
    FstringStart, FstringMiddle, FstringEnd,
    // Whitespace / structure
    Comment, Newline, Indent, Dedent, Nl, Endmarker
}

/*
Token Stream
    Produces a parser-ready iterator with indentation and soft keywords resolved.
*/

pub fn lexer(source: &str) -> impl Iterator<Item = Token> + '_ {
    let bytes = source.as_bytes();
    let len = source.len();
    let mut scanner = Scanner::new(bytes);
    let mut done = false;

    if len > MAX_SOURCE_SIZE {
        scanner.pending.push((TokenType::Endmarker, 0, len, len));
    }

    let mut stream = core::iter::from_fn(move || {
        if done { return Option::None; }
        match scanner.next_token() {
            Some(tok) => Some(tok),
            Option::None if !done => {
                done = true;
                Some((TokenType::Endmarker, scanner.line, len, len))
            }
            _ => Option::None
        }
    })
    .peekable();

    let mut ended = false;

    core::iter::from_fn(move || {
        let (tok, line, start, end) = stream.next()?;

        if ended { return Option::None; }
        if tok == TokenType::Endmarker { ended = true; }

        let is_soft = matches!(tok, TokenType::Match | TokenType::Case | TokenType::Type);
        let next_demotes = matches!(
            stream.peek(),
            Some((
                TokenType::Lpar | TokenType::Colon | TokenType::Equal | TokenType::Comma
                    | TokenType::Rpar | TokenType::Rsqb | TokenType::Newline,
                _, _, _
            )) | Option::None
        );

        let kind = if is_soft && next_demotes { TokenType::Name } else { tok };
        Some(Token { kind, line, start, end })
    })
}