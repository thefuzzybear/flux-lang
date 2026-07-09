//! Token definitions for Flux lexer

/// Flux tokens
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // Keywords
    Strategy,
    Params,
    State,
    On,
    If,
    Elif,
    Else,
    For,
    While,
    Return,
    Fn,
    From,
    Import,
    And,
    Or,
    Not,
    True,
    False,
    Null,
    Data,
    Connector,
    Struct,

    // Identifiers and literals
    Ident(String),
    Int(i64),
    Float(f64),
    String(String),

    // Operators
    Plus,       // +
    Minus,      // -
    Star,       // *
    Slash,      // /
    Percent,    // %
    Eq,         // ==
    Ne,         // !=
    Lt,         // <
    Le,         // <=
    Gt,         // >
    Ge,         // >=
    AndAnd,     // &&
    OrOr,       // ||
    Bang,       // !
    At,         // @
    Arrow,      // ->

    // Delimiters
    OpenParen,   // (
    CloseParen,  // )
    OpenBrace,   // {
    CloseBrace,  // }
    OpenBracket, // [
    CloseBracket,// ]
    Comma,       // ,
    Dot,         // .
    Colon,       // :
    ColonColon,  // ::
    Semicolon,   // ;
    Assign,      // =

    // Special
    Eof,
}

impl Token {
    /// Check if token is a keyword
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            Token::Strategy
                | Token::Params
                | Token::State
                | Token::On
                | Token::If
                | Token::Elif
                | Token::Else
                | Token::For
                | Token::While
                | Token::Return
                | Token::Fn
                | Token::From
                | Token::Import
                | Token::And
                | Token::Or
                | Token::Not
                | Token::True
                | Token::False
                | Token::Null
                | Token::Data
                | Token::Connector
                | Token::Struct
        )
    }
}
