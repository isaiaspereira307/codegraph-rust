package shapes

import "sync"

type Circle struct {
	Shape
	Radius int
}

type Guarded struct {
	sync.Mutex
}
