require_relative "test_helper"

class EmailNormalizationTest < Minitest::Test
  def test_login_is_case_insensitive
    auth = Authenticator.new(clock: Clock.new)
    auth.register(email: "Ada@Example.com", password: "correct horse")
    assert auth.login(email: "ada@example.COM ", password: "correct horse")
  end
end
