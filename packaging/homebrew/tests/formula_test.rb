require "pathname"

ROOT = `git rev-parse --show-toplevel`.strip
FORMULA_PATH = File.join(ROOT, "packaging", "homebrew", "lite.rb")
README_PATH = File.join(ROOT, "packaging", "homebrew", "README.md")
CLI_MANIFEST_PATH = File.join(ROOT, "crates", "betterlite-cli", "Cargo.toml")

def assert(condition, message)
  raise message unless condition
end

def build_stub
  Class.new do
    class << self
      attr_reader :desc_value, :homepage_value, :url_value, :sha256_value,
                  :license_value, :depends_on_calls, :test_block

      def desc(value)
        @desc_value = value
      end

      def homepage(value)
        @homepage_value = value
      end

      def url(value)
        @url_value = value
      end

      def sha256(value)
        @sha256_value = value
      end

      def license(value)
        @license_value = value
      end

      def depends_on(*arguments)
        @depends_on_calls ||= []
        @depends_on_calls << arguments
      end

      def test(&block)
        @test_block = block
      end
    end

    attr_reader :system_calls, :shell_output_calls, :assert_match_calls

    def initialize
      @system_calls = []
      @shell_output_calls = []
      @assert_match_calls = []
    end

    def std_cargo_args(path:)
      ["--jobs", "1", "--locked", "--root", "/stub/prefix", "--path", path]
    end

    def system(*arguments)
      @system_calls << arguments
    end

    def bin
      Pathname.new("/stub/bin")
    end

    def buildpath
      Pathname.new("/stub/buildpath")
    end

    def libexec
      Pathname.new("/stub/libexec")
    end

    def shell_output(command)
      @shell_output_calls << command
      "Usage: lite [OPTIONS] COMMAND\nlogin\n"
    end

    def assert_match(pattern, subject)
      @assert_match_calls << [pattern, subject]
    end
  end
end

def formula_class
  @formula_class ||= begin
    Object.send(:remove_const, :Lite) if Object.const_defined?(:Lite)
    Object.send(:remove_const, :Formula) if Object.const_defined?(:Formula)
    @formula_stub = build_stub
    Object.const_set(:Formula, @formula_stub)
    load FORMULA_PATH
    Lite
  end
end

def crate_declares_lite_bin?
  in_bin = false
  File.readlines(CLI_MANIFEST_PATH, chomp: true).any? do |line|
    stripped = line.strip
    in_bin = false if stripped.start_with?("[") && stripped != "[[bin]]"
    in_bin = true if stripped == "[[bin]]"
    in_bin && stripped == 'name = "lite"'
  end
end

def case_syntax
  RubyVM::InstructionSequence.compile_file(FORMULA_PATH)
end

def case_metadata
  klass = formula_class
  assert klass.name == "Lite", "the formula class is named #{klass.name.inspect}"
  assert klass.superclass.equal?(@formula_stub), "the formula class does not inherit from Formula"

  desc = klass.desc_value
  assert desc.is_a?(String), "desc is missing"
  assert desc.length.between?(1, 79), "desc is #{desc.length} characters, not 1 to 79"
  assert !desc.end_with?("."), "desc ends with a period"
  assert !desc.start_with?("A ", "An ", "The "), "desc starts with an article"
  assert desc.include?("lite"), "desc does not mention lite"

  homepage = klass.homepage_value
  assert homepage == "https://github.com/betterlite-ai/betterlite", "homepage is #{homepage.inspect}"
  assert klass.license_value == "MIT", "license is #{klass.license_value.inspect}"
  assert klass.depends_on_calls == [[{ "rust" => :build }]], "depends_on was called as #{klass.depends_on_calls.inspect}"

  url = klass.url_value
  assert url.is_a?(String), "url is missing"
  assert url.match?(%r{\Ahttps://github\.com/betterlite-ai/betterlite/archive/refs/tags/v.+\.tar\.gz\z}), "url is #{url.inspect}"
end

def case_install
  formula = formula_class.new
  formula.install
  expected = ["cargo", "install", "--jobs", "1", "--locked", "--root", "/stub/prefix", "--path", "crates/betterlite-cli"]
  assert formula.system_calls == [expected], "install ran #{formula.system_calls.inspect}"
  assert File.file?(CLI_MANIFEST_PATH), "crates/betterlite-cli/Cargo.toml does not exist"
  assert crate_declares_lite_bin?, 'crates/betterlite-cli/Cargo.toml has no [[bin]] with name = "lite"'
end

def case_test_block
  klass = formula_class
  block = klass.test_block
  assert block.is_a?(Proc), "the formula has no test block"
  instance = klass.new
  instance.instance_exec(&block)
  assert instance.shell_output_calls == ["/stub/bin/lite --help"], "the test block ran #{instance.shell_output_calls.inspect}"
  assert instance.assert_match_calls == [["login", "Usage: lite [OPTIONS] COMMAND\nlogin\n"]], "the test block asserted #{instance.assert_match_calls.inspect}"
end

def case_release_lockstep
  klass = formula_class
  url = klass.url_value
  checksum = klass.sha256_value
  assert url.is_a?(String), "url is missing"
  assert checksum.is_a?(String), "sha256 is missing"

  placeholder = url.include?("vREPLACE_AT_RELEASE") && checksum == "REPLACE_AT_RELEASE"
  released = url.match?(%r{/archive/refs/tags/v\d+\.\d+\.\d+\.tar\.gz\z}) && checksum.match?(/\A[0-9a-f]{64}\z/)
  assert placeholder || released, "url #{url.inspect} and sha256 #{checksum.inspect} are not both placeholders or both released"
end

def case_no_python
  text = File.read(FORMULA_PATH)
  ["python", "pip", "virtualenv", "curl", "wget"].each do |word|
    assert !text.match?(/#{word}/i), "lite.rb mentions #{word}"
  end
  assert !text.match?(/git clone/i), "lite.rb runs git clone"
end

def case_readme
  readme = File.read(README_PATH)
  [
    "brew install betterlite-ai/betterlite/lite",
    "betterlite-ai/homebrew-betterlite",
    "homebrew-core",
    "curl -fsSL https://github.com/betterlite-ai/betterlite/archive/refs/tags/vX.Y.Z.tar.gz | sha256sum",
    "brew install --build-from-source ./Formula/lite.rb",
    "brew test lite",
    "brew audit --strict --new lite",
    "Steps 4 and 5 are the only proof of a real Homebrew build and need a published tag, so they are not part of CI."
  ].each do |fragment|
    assert readme.include?(fragment), "README.md does not contain #{fragment.inspect}"
  end
end

CASES = {
  "syntax" => :case_syntax,
  "metadata" => :case_metadata,
  "install" => :case_install,
  "test_block" => :case_test_block,
  "release_lockstep" => :case_release_lockstep,
  "no_python" => :case_no_python,
  "readme" => :case_readme
}.freeze

def run_case(name)
  send(CASES.fetch(name) { raise "unknown case #{name}" })
  true
rescue StandardError, ScriptError => error
  warn "#{name}: #{error.message}"
  false
end

names = ARGV.empty? ? CASES.keys : [ARGV.first]
failed = names.reject { |name| run_case(name) }
exit 1 unless failed.empty?

puts "ok"
