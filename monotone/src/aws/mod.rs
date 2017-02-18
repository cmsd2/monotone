pub mod counter;
pub mod dynamodb;
pub mod error;

#[derive(Serialize, Deserialize)]
pub struct AWSError {
    #[serde(rename="__type")]
    pub typ: String,
    pub message: String,
}