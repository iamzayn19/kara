require "digest"

# A user with a salted password digest.
class User
  attr_reader :email, :password_digest

  def initialize(email:, password:)
    @email = normalize(email)
    @password_digest = digest(password)
  end

  def authenticate(password)
    digest(password) == password_digest
  end

  def self.normalize_email(email)
    email.to_s.strip
  end

  private

  def normalize(email)
    User.normalize_email(email)
  end

  def digest(password)
    Digest::SHA256.hexdigest("veyra-fixture-salt:#{password}")
  end
end
