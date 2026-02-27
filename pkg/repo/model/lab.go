package model

import "github.com/scienceol/osdl/pkg/common/uuid"

type UserData struct {
	Owner        string    `json:"owner"`
	Name         string    `json:"name"`
	ID           string    `json:"id"`
	OrgID        string    `json:"org_id,omitempty"`
	Avatar       string    `json:"avatar"`
	Type         string    `json:"type"`
	DisplayName  string    `json:"displayName"`
	Email        string    `json:"email"`
	Phone        string    `json:"phone"`
	AccessKey    string    `json:"access_key,omitempty"`
	AccessSecret string    `json:"access_secret,omitempty"`
	LabID        int64     `json:"lab_id,omitempty"`
	LabUUID      uuid.UUID `json:"lab_uuid,omitempty"`
}

type UserInfo struct {
	Status string    `json:"status"`
	Msg    string    `json:"msg"`
	Data   *UserData `json:"data"`
}

type LabAkSk struct {
	AccessKey    string `json:"access_key"`
	AccessSecret string `json:"access_secret"`
}
