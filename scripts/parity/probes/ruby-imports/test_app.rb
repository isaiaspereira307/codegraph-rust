require_relative 'app'

module Demo
  class AppTest
    def test_run
      App.new.run(Helper.new)
    end
  end
end
