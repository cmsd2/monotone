
pub fn s<S>(string: S) -> String where S: Into<String> {
    string.into()
}