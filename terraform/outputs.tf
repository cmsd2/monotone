output "test_user_access_key_id" {
  description = "Access key ID of the test user"
  value       = aws_iam_access_key.monotone_test.id
}

output "test_user_encrypted_secret" {
  description = "Secret access key of the test user, encrypted with pgp_key and base64-encoded"
  value       = aws_iam_access_key.monotone_test.encrypted_secret
}
