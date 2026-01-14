package invite

import (
	// 内部引用
	repo "github.com/scienceol/opensdl/service/pkg/repo"
)

type inviteImpl struct {
	repo.IDOrUUIDTranslate
}

func New() repo.Invite {
	return &inviteImpl{
		IDOrUUIDTranslate: repo.NewBaseDB(),
	}
}
