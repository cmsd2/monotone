resource "aws_iam_user" "travis_monotone" {
    name = "travis_monotone"
    path = "/ci/"
}

resource "aws_iam_access_key" "travis_monotone" {
    user = "${aws_iam_user.travis_monotone.name}"
    pgp_key = "${var.pgp_key}"
}

resource "aws_iam_user_policy" "travis_monotone" {
    name = "travis_monotone"
    user = "${aws_iam_user.travis_monotone.name}"
    policy = "${data.template_file.travis_monotone_policy.rendered}"
}

data "template_file" "travis_monotone_policy" {
    template = "${file("${path.module}/policy.json")}"

    vars {
        monotone_table = "${var.monotone_table}"
        monotone_region = "${var.monotone_table_region}"
        aws_account = "${var.aws_account}"
    }
}