package casdoor

import (
	// 外部依赖
	"context"
	"errors"
	"net/http"

	resty "github.com/go-resty/resty/v2"
	gorm "gorm.io/gorm"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"

)

type casClient struct {
	casDoorClient *resty.Client
	repo.IDOrUUIDTranslate
}

func NewCasClient() repo.Account {
	conf := config.Global().OAuth2
	return &casClient{
		IDOrUUIDTranslate: repo.NewBaseDB(),
		casDoorClient: resty.New().
			EnableTrace().
			SetBaseURL(conf.Addr),
	}
}

func (c *casClient) CreateLabUser(ctx context.Context, user *model.LabInfo) error {
	resData := &model.LabInfoResp{}
	conf := config.Global().OAuth2
	resp, err := c.casDoorClient.R().SetContext(ctx).
		SetBody(user).
		SetResult(resData).
		SetBasicAuth(conf.ClientID, conf.ClientSecret).
		SetResult(nil).Post("/api/add-user")
	if err != nil {
		logger.Errorf(ctx, "CreateLabUser err: %+v user: %+v", err, user)
		return code.CasDoorCreateLabUserErr.WithMsg(err.Error())
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "CreateLabUser http code: %d", resp.StatusCode())
		return code.CasDoorCreateLabUserErr
	}

	if resData.Status != "ok" {
		logger.Errorf(ctx, "CreateLabUser res data err: %+v", resData)
		return code.CasDoorCreateLabUserErr
	}

	return nil
}

func (c *casClient) GetLabUserInfo(ctx context.Context, req *model.LabAkSk) (*model.UserData, error) {
	// 实验室用户就是创建该实验室的人
	labData := &model.Laboratory{}
	if err := c.DBWithContext(ctx).
		Where("access_key = ? and access_secret = ?",
			req.AccessKey, req.AccessSecret).
		Select("id", "user_id").
		Take(labData).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			return nil, code.RecordNotFound
		}

		logger.Errorf(ctx, "GetLabUserInfo fail err: %+v", err)
		return nil, err
	}

	return &model.UserData{
		ID:    labData.UserID,
		LabID: labData.ID,
	}, nil
}

func (c *casClient) BatchGetUserInfo(ctx context.Context, uesrIDs []string) ([]*model.UserData, error) {
	panic("not impl")
}

func (c *casClient) GetUserInfo(ctx context.Context, userID string) (*model.UserData, error) {
	panic("not impl")
}

func (c *casClient) GetLabUserByAccessKey(ctx context.Context, accessKey string) (*model.UserData, error) {
	panic("not impl")
}
