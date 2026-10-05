require_relative "../../config/settings"

# A login session that expires after Settings::SESSION_TTL_SECONDS.
class Session
  attr_reader :user, :token, :created_at

  def initialize(user:, token:, created_at:)
    @user = user
    @token = token
    @created_at = created_at
  end

  def expired?(now)
    now - created_at >= Settings::SESSION_TTL_SECONDS * 60
  end
end
