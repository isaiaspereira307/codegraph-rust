package app

import (
	"example.com/probe/pkg/helper"
	"example.com/dep"
	"example.com/vendored"
)

func Run() {
	helper.Help()
	dep.Dep()
	vendored.Value()
}
