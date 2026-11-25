module example.com/probe

go 1.21

require (
	example.com/dep v0.0.0
	example.com/vendored v0.0.0
)

replace example.com/dep => ./third_party/dep
