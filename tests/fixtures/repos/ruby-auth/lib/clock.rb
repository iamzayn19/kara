# Injectable clock so tests can control time.
class Clock
  attr_accessor :now

  def initialize(now = Time.at(0))
    @now = now
  end

  def advance(seconds)
    @now += seconds
  end
end
