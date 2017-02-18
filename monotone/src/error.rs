error_chain! {
    errors {
        NotFound(process_id: String) {
            description("ticket not found")
            display("ticket not found for process_id {}", process_id)
        }
    }
}