resource "aws_iam_user" "monotone_test" {
  name = "monotone_test"
  path = "/test/"
}

resource "aws_iam_access_key" "monotone_test" {
  user    = aws_iam_user.monotone_test.name
  pgp_key = var.pgp_key
}

resource "aws_iam_user_policy" "monotone_test" {
  name = "monotone_test"
  user = aws_iam_user.monotone_test.name
  policy = templatefile("${path.module}/policy.json", {
    monotone_table  = var.monotone_table
    monotone_region = var.monotone_table_region
    aws_account     = var.aws_account
  })
}
