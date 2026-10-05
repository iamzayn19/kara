require "securerandom"
require_relative "../models/user"
require_relative "../models/session"

# Logs users in and resolves session tokens.
class Authenticator
  def initialize(clock:)
    @clock = clock
    @users = {}
    @sessions = {}
  end

  def register(email:, password:)
    user = User.new(email: email, password: password)
    @users[user.email] = user
    user
  end

  def login(email:, password:)
    user = @users[User.normalize_email(email)]
    return nil unless user&.authenticate(password)

    token = SecureRandom.hex(16)
    @sessions[token] = Session.new(user: user, token: token, created_at: @clock.now)
    token
  end

  def current_user(token)
    session = @sessions[token]
    return nil if session.nil? || session.expired?(@clock.now)

    session.user
  end
end
