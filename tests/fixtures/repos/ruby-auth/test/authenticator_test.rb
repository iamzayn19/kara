require_relative "test_helper"

class AuthenticatorTest < Minitest::Test
  def setup
    @clock = Clock.new
    @auth = Authenticator.new(clock: @clock)
    @auth.register(email: "ada@example.com", password: "correct horse")
  end

  def test_login_with_valid_password
    assert @auth.login(email: "ada@example.com", password: "correct horse")
  end

  def test_login_with_wrong_password
    assert_nil @auth.login(email: "ada@example.com", password: "nope")
  end

  def test_session_resolves_user
    token = @auth.login(email: "ada@example.com", password: "correct horse")
    assert_equal "ada@example.com", @auth.current_user(token).email
  end

  def test_session_expires_after_thirty_minutes
    token = @auth.login(email: "ada@example.com", password: "correct horse")
    @clock.advance(29 * 60)
    refute_nil @auth.current_user(token)
    @clock.advance(60)
    assert_nil @auth.current_user(token)
  end
end
