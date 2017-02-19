output "travis_user_secret" {
    value = "${aws_iam_access_key.travis_monotone.encrypted_secret}"
}