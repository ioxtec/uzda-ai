pub fn welcome_message() -> &'static str {
    "Welcome to Uzda!"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_welcome_message() {
        assert_eq!(welcome_message(), "Welcome to Uzda!");
    }
}
