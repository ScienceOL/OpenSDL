package common

type Role string

const (
	SuperAdmin Role = "super_admin"
	Admin      Role = "admin"
	Normal     Role = "normal"
)

type Perm string

const (
	Read      Perm = "read"
	Create    Perm = "create"
	Update    Perm = "update"
	Delete    Perm = "delete"
	Visible   Perm = "visible"
	Clickable Perm = "clickable"
)
