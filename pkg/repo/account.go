package repo

import (
	"context"

	"github.com/scienceol/osdl/pkg/repo/model"
)

type Account interface {
	GetLabUserInfo(ctx context.Context, aksk *model.LabAkSk) (*model.UserData, error)
	GetLabUserByAccessKey(ctx context.Context, accessKey string) (*model.UserData, error)
}

type LabAccount interface {
	GetLabUserInfo(ctx context.Context, aksk *model.LabAkSk) (*model.UserData, error)
}
